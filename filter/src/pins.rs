//! Input and output pins of the FFmpeg transform filter.

use std::sync::Arc;

use windows::core::*;
use windows::Win32::Media::DirectShow::*;
use windows::Win32::Media::MediaFoundation::{AM_MEDIA_TYPE, MEDIATYPE_Video};
use windows::Win32::System::Com::CoTaskMemAlloc;

use crate::alloc::MemAllocator;
use crate::enums::EnumMediaTypes;
use crate::mediatype;
use crate::state::{
    debug_log, err, E_FAIL, E_NOTIMPL_HR, E_OUTOFMEMORY, E_POINTER, E_UNEXPECTED_HR, S_FALSE_HR,
    Shared,
};

fn fill_ach_name(dst: &mut [u16; 128], name: &str) {
    let n = name.encode_utf16().take(127).collect::<Vec<u16>>();
    for (i, ch) in n.iter().enumerate() {
        dst[i] = *ch;
    }
    dst[n.len()] = 0;
}

fn query_id(name: &str) -> Result<PWSTR> {
    let mut v: Vec<u16> = name.encode_utf16().collect();
    v.push(0);
    let p = unsafe { CoTaskMemAlloc(v.len() * 2) } as *mut u16;
    if p.is_null() {
        return Err(err(E_OUTOFMEMORY));
    }
    unsafe {
        std::ptr::copy_nonoverlapping(v.as_ptr(), p, v.len());
    }
    Ok(PWSTR(p))
}

fn hr_of(r: &Result<()>) -> i32 {
    match r {
        Ok(_) => 0,
        Err(e) => e.code().0,
    }
}

// ---------------------------------------------------------------------------
// Deliver one encoded packet downstream through the output connection.
// ---------------------------------------------------------------------------

fn deliver_packet(shared: &Shared, data: Vec<u8>) -> Result<()> {
    if data.is_empty() {
        return Ok(());
    }
    debug_log(&format!("deliver_packet size={}", data.len()));
    let (start, stop, flags, first) = {
        let mut core = shared.core.lock().unwrap();
        let frame_dur = core.fmt.as_ref().map(|f| f.frame_dur).unwrap_or(1);
        let start = core.ts_queue.pop_front().unwrap_or(core.last_ts + frame_dur);
        core.last_ts = start;
        let stop = start + frame_dur;
        let flags = if core.first_pkt { 0 } else { AM_GBF_NOTASYNCPOINT };
        let first = core.first_pkt;
        core.first_pkt = false;
        (start, stop, flags, first)
    };

    let conn = shared.output.lock().unwrap();
    let allocator = conn.allocator.as_ref().ok_or(err(VFW_E_NOT_CONNECTED))?;
    let meminput = conn.meminput.as_ref().ok_or(err(VFW_E_NOT_CONNECTED))?;
    // Never write past the downstream allocator's buffers: that corrupts the
    // AVI Mux heap and can surface later as a NULL deref inside quartz.
    let cb_buffer = conn.cb_buffer;
    if cb_buffer > 0 && data.len() as i32 > cb_buffer {
        crate::state::always_log(&format!(
            "deliver_packet dropped {} bytes (downstream buffer is {})",
            data.len(),
            cb_buffer
        ));
        return Ok(());
    }

    let mut sample: Option<IMediaSample> = None;
    unsafe {
        match allocator.GetBuffer(&mut sample, Some(&start), Some(&stop), flags) {
            Ok(_) => {}
            Err(e) => {
                debug_log(&format!("deliver_packet GetBuffer failed {:08X}", e.code().0));
                return Err(e);
            }
        }
    }
    let sample = match sample {
        Some(s) => s,
        None => {
            debug_log("deliver_packet GetBuffer returned None");
            return Err(err(E_POINTER));
        }
    };
    // The negotiated cb_buffer is only a request; the allocator may have
    // returned a smaller actual buffer (the AVI Mux can renegotiate after
    // NotifyAllocator). Copying past GetSize() corrupts the heap and shows
    // up later as a NULL deref inside quartz, so always verify the sample.
    let sample_size = unsafe { sample.GetSize() };
    if sample_size < 0 || data.len() > sample_size as usize {
        crate::state::always_log(&format!(
            "deliver_packet dropped {} bytes (actual sample buffer is {})",
            data.len(),
            sample_size
        ));
        return Ok(());
    }
    let ptr = unsafe { sample.GetPointer()? };
    unsafe {
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
    }
    unsafe {
        sample.SetTime(Some(&start), Some(&stop))?;
        sample.SetActualDataLength(data.len() as i32)?;
        if first {
            sample.SetSyncPoint(true)?;
            sample.SetDiscontinuity(true)?;
        }
        let r = meminput.Receive(&sample);
        debug_log(&format!("deliver_packet Receive hr={:08X}", hr_of(&r)));
        r
    }
}

fn drain_deliver(shared: &Shared) {
    let pkts = shared
        .encoder
        .lock()
        .unwrap()
        .as_ref()
        .map(|e| e.drain())
        .unwrap_or_default();
    debug_log(&format!("drain_deliver packets={}", pkts.len()));
    for p in pkts {
        let r = deliver_packet(shared, p);
        debug_log(&format!("drain_deliver result {:08X}", hr_of(&r)));
    }
}

/// Called from Filter::Run once the encoder is ready: write any frames that
/// arrived earlier, then deliver their packets and propagate a deferred
/// EndOfStream if one was received before the encoder started.
pub fn flush_pending(shared: &Shared) -> Result<()> {
    let (frames, eos) = {
        let mut core = shared.core.lock().unwrap();
        let frames = core.pending_frames.drain(..).collect::<Vec<_>>();
        let eos = core.eos_pending;
        core.eos_pending = false;
        (frames, eos)
    };
    debug_log(&format!(
        "flush_pending frames={} eos={}",
        frames.len(),
        eos
    ));
    {
        let mut enc = shared.encoder.lock().unwrap();
        if let Some(e) = enc.as_mut() {
            for f in &frames {
                if let Err(ioe) = e.write_frame(f) {
                    if !crate::state::report_failure(&format!(
                        "flush_pending write_frame failed: {}",
                        ioe
                    )) {
                        crate::state::debug_log("flush_pending write_frame failed (already reported)");
                    }
                    // The encoder is dead. Drop the remaining buffered
                    // frames instead of failing the graph, so MMD does not
                    // hang on an error it may not handle.
                    break;
                }
            }
            if eos {
                e.flush();
                e.wait_reader(10000);
            }
        }
    }
    drain_deliver(shared);
    if eos {
        send_eos_to_peer(shared)?;
    }
    Ok(())
}

/// Forward EndOfStream to the downstream peer exactly once. Some sources
/// (MMD's DIB Sequential + SampleGrabber chain) can deliver EOS multiple
/// times; a second EOS into the AVI Mux is invalid and has been observed to
/// crash quartz with a NULL dereference.
fn send_eos_to_peer(shared: &Shared) -> Result<()> {
    {
        let core = shared.core.lock().unwrap();
        if core.eos_sent {
            return Ok(());
        }
    }
    let out = shared.output.lock().unwrap();
    if let Some(peer) = out.peer.as_ref() {
        unsafe {
            peer.EndOfStream()?;
        }
    }
    shared.core.lock().unwrap().eos_sent = true;
    Ok(())
}

// ---------------------------------------------------------------------------
// Input pin (IPin + IMemInputPin)
// ---------------------------------------------------------------------------

#[implement(IPin, IMemInputPin)]
pub struct InputPin {
    shared: Arc<Shared>,
}

impl InputPin {
    pub fn new(shared: Arc<Shared>) -> Self {
        InputPin { shared }
    }
}

impl IPin_Impl for InputPin_Impl {
    fn Connect(&self, _preceivepin: Ref<'_, IPin>, _pmt: *const AM_MEDIA_TYPE) -> Result<()> {
        debug_log("InputPin::Connect called (unexpected)");
        Err(err(E_UNEXPECTED_HR))
    }

    fn ReceiveConnection(&self, pconnector: Ref<'_, IPin>, pmt: *const AM_MEDIA_TYPE) -> Result<()> {
        crate::state::always_log(&format!(
            "InputPin::ReceiveConnection pmt_null={}",
            pmt.is_null()
        ));
        if let Some(peer) = pconnector.cloned() {
            let mut info = PIN_INFO::default();
            if unsafe { peer.QueryPinInfo(&mut info) }.is_ok() {
                let mut name = String::new();
                for ch in info.achName.iter() {
                    if *ch == 0 {
                        break;
                    }
                    name.push(char::from_u32(*ch as u32).unwrap_or('?'));
                }
                debug_log(&format!(
                    "InputPin::ReceiveConnection connector dir={} name={}",
                    info.dir.0, name
                ));
            }
        }
        let mut chosen = if pmt.is_null() {
            let mut accepted = None;
            let e = unsafe { pconnector.unwrap().EnumMediaTypes() }?;
            loop {
                let mut arr = [std::ptr::null_mut::<AM_MEDIA_TYPE>(); 1];
                let hr = unsafe { e.Next(&mut arr, None) };
                let p = arr[0];
                if hr.0 != 0 || p.is_null() {
                    break;
                }
                let mt = unsafe { &*p };
                if mediatype::check_input(mt) {
                    accepted = Some(mediatype::clone_mt(mt));
                }
                unsafe {
                    mediatype::free_mt(&mut *p);
                    windows::Win32::System::Com::CoTaskMemFree(Some(p as *const _));
                }
                if accepted.is_some() {
                    break;
                }
            }
            accepted.ok_or(err(VFW_E_NO_ACCEPTABLE_TYPES))?
        } else {
            let mt = unsafe { pmt.as_ref() }.ok_or(err(E_POINTER))?;
            if !mediatype::check_input(mt) {
                debug_log(&format!(
                    "InputPin::ReceiveConnection reject major={:08X} sub={:08X} fmttype={:08X}",
                    mt.majortype.data1,
                    mt.subtype.data1,
                    mt.formattype.data1
                ));
                return Err(err(VFW_E_TYPE_NOT_ACCEPTED));
            }
            mediatype::clone_mt(mt)
        };
        debug_log("InputPin::ReceiveConnection accepted");

        let mut conn = self.shared.input.lock().unwrap();
        if conn.peer.is_some() {
            unsafe { mediatype::free_mt(&mut chosen) };
            return Err(err(VFW_E_ALREADY_CONNECTED));
        }
        if let Some(fmt) = mediatype::parse_input(&chosen) {
            self.shared.core.lock().unwrap().fmt = Some(fmt);
        }
        debug_log(&format!(
            "InputPin::ReceiveConnection type sub={:08X} fmttype={:08X} cb={} fixed={} lsample={}",
            chosen.subtype.data1,
            chosen.formattype.data1,
            chosen.cbFormat,
            chosen.bFixedSizeSamples.0,
            chosen.lSampleSize
        ));
        if !chosen.pbFormat.is_null() && chosen.cbFormat > 0 {
            let p = chosen.pbFormat as *const u8;
            let mut hex = String::new();
            let n = (chosen.cbFormat as usize).min(64);
            for k in 0..n {
                hex.push_str(&format!("{:02X} ", unsafe { *p.add(k) }));
            }
            debug_log(&format!(
                "InputPin::ReceiveConnection fmt[0..{}]={}",
                n.saturating_sub(1),
                hex
            ));
        }
        conn.peer = pconnector.cloned();
        conn.mt = Some(chosen);
        Ok(())
    }

    fn Disconnect(&self) -> Result<()> {
        crate::state::always_log("InputPin::Disconnect");
        let mut conn = self.shared.input.lock().unwrap();
        if conn.peer.is_none() {
            return Err(err(S_FALSE_HR));
        }
        conn.clear();
        Ok(())
    }

    fn ConnectedTo(&self) -> Result<IPin> {
        crate::state::always_log("InputPin::ConnectedTo");
        self.shared
            .input
            .lock()
            .unwrap()
            .peer
            .clone()
            .ok_or(err(VFW_E_NOT_CONNECTED))
    }

    fn ConnectionMediaType(&self, pmt: *mut AM_MEDIA_TYPE) -> Result<()> {
        crate::state::always_log("InputPin::ConnectionMediaType");
        let conn = self.shared.input.lock().unwrap();
        let mt = conn.mt.as_ref().ok_or(err(VFW_E_NOT_CONNECTED))?;
        if pmt.is_null() {
            return Err(err(E_POINTER));
        }
        unsafe {
            *pmt = mediatype::clone_mt(mt);
        }
        Ok(())
    }

    fn QueryPinInfo(&self, pinfo: *mut PIN_INFO) -> Result<()> {
        crate::state::always_log("InputPin::QueryPinInfo");
        if pinfo.is_null() {
            return Err(err(E_POINTER));
        }
        let mut info = PIN_INFO {
            dir: PINDIR_INPUT,
            ..Default::default()
        };
        fill_ach_name(&mut info.achName, "FFmpegIn");
        info.pFilter = core::mem::ManuallyDrop::new(self.shared.filter_iface());
        unsafe {
            pinfo.write(info);
        }
        Ok(())
    }

    fn QueryDirection(&self) -> Result<PIN_DIRECTION> {
        crate::state::always_log("InputPin::QueryDirection");
        Ok(PINDIR_INPUT)
    }

    fn QueryId(&self) -> Result<PWSTR> {
        crate::state::always_log("InputPin::QueryId");
        query_id("FFmpegIn")
    }

    fn QueryAccept(&self, pmt: *const AM_MEDIA_TYPE) -> HRESULT {
        crate::state::always_log("InputPin::QueryAccept");
        let Some(mt) = (unsafe { pmt.as_ref() }) else {
            return E_POINTER;
        };
        if mediatype::check_input(mt) {
            debug_log("InputPin::QueryAccept OK");
            HRESULT(0)
        } else {
            debug_log("InputPin::QueryAccept reject");
            VFW_E_TYPE_NOT_ACCEPTED
        }
    }

    fn EnumMediaTypes(&self) -> Result<IEnumMediaTypes> {
        crate::state::always_log("InputPin::EnumMediaTypes");
        Ok(EnumMediaTypes::new(Vec::new()).into())
    }

    fn QueryInternalConnections(&self, appin: OutRef<'_, IPin>, npin: *mut u32) -> Result<()> {
        crate::state::always_log("InputPin::QueryInternalConnections");
        // A normal transform filter has no hidden internal connections;
        // returning E_NOTIMPL (like the DirectShow base classes) keeps the
        // graph builder from trying to "complete" a path through us and
        // creating circular graphs.
        let _ = (appin, npin);
        Err(err(E_NOTIMPL_HR))
    }

    fn EndOfStream(&self) -> Result<()> {
        debug_log("InputPin::EndOfStream");
        {
            let mut core = self.shared.core.lock().unwrap();
            core.completed = true;
            if core.eos_sent {
                debug_log("InputPin::EndOfStream ignored (already sent)");
                return Ok(());
            }
        }
        let has_encoder = self.shared.encoder.lock().unwrap().is_some();
        if !has_encoder {
            // The encoder is still starting (auto-detection/probing can take
            // a couple of seconds). Hold the EOS until Run() flushes the
            // buffered frames, otherwise the AVI Mux would finalize with an
            // empty file.
            self.shared.core.lock().unwrap().eos_pending = true;
            debug_log("InputPin::EndOfStream deferred (encoder not ready)");
            return Ok(());
        }
        {
            let mut enc = self.shared.encoder.lock().unwrap();
            if let Some(e) = enc.as_mut() {
                e.flush();
                e.wait_reader(10000);
                let pkts = e.drain();
                drop(enc);
                for p in pkts {
                    let _ = deliver_packet(&self.shared, p);
                }
            }
        }
        send_eos_to_peer(&self.shared)
    }

    fn BeginFlush(&self) -> Result<()> {
        debug_log("InputPin::BeginFlush");
        {
            let mut enc = self.shared.encoder.lock().unwrap();
            if let Some(mut e) = enc.take() {
                e.stop();
            }
        }
        {
            let mut core = self.shared.core.lock().unwrap();
            core.ts_queue.clear();
            core.first_pkt = true;
            core.pending_frames.clear();
            core.eos_pending = false;
            core.eos_sent = false;
        }
        let out = self.shared.output.lock().unwrap();
        if let Some(peer) = out.peer.as_ref() {
            unsafe {
                peer.BeginFlush()?;
            }
        }
        Ok(())
    }

    fn EndFlush(&self) -> Result<()> {
        debug_log("InputPin::EndFlush");
        let out = self.shared.output.lock().unwrap();
        if let Some(peer) = out.peer.as_ref() {
            unsafe {
                peer.EndFlush()?;
            }
        }
        Ok(())
    }

    fn NewSegment(&self, tstart: i64, tstop: i64, drate: f64) -> Result<()> {
        debug_log(&format!("InputPin::NewSegment {}-{} rate={}", tstart, tstop, drate));
        self.shared.core.lock().unwrap().eos_sent = false;
        let out = self.shared.output.lock().unwrap();
        if let Some(peer) = out.peer.as_ref() {
            unsafe {
                peer.NewSegment(tstart, tstop, drate)?;
            }
        }
        Ok(())
    }
}

impl IMemInputPin_Impl for InputPin_Impl {
    fn GetAllocator(&self) -> Result<IMemAllocator> {
        crate::state::always_log("InputPin::GetAllocator");
        Err(err(E_NOTIMPL_HR))
    }

    fn NotifyAllocator(&self, pallocator: Ref<'_, IMemAllocator>, _breadonly: BOOL) -> Result<()> {
        crate::state::always_log("InputPin::NotifyAllocator");
        self.shared.input.lock().unwrap().allocator = pallocator.cloned();
        Ok(())
    }

    fn GetAllocatorRequirements(&self) -> Result<ALLOCATOR_PROPERTIES> {
        crate::state::always_log("InputPin::GetAllocatorRequirements");
        Err(err(E_NOTIMPL_HR))
    }

    fn Receive(&self, psample: Ref<'_, IMediaSample>) -> Result<()> {
        let sample = psample.cloned().ok_or(err(E_POINTER))?;
        let len = unsafe { sample.GetActualDataLength() };
        debug_log(&format!("InputPin::Receive len={}", len));
        if len <= 0 {
            return Ok(());
        }
        let ptr = unsafe { sample.GetPointer()? };
        let data = unsafe { std::slice::from_raw_parts(ptr, len as usize) };

        let mut core = self.shared.core.lock().unwrap();
        if !core.started {
            debug_log("InputPin::Receive dropped: !started");
            return Ok(());
        }
        let fmt = match core.fmt.as_ref() {
            Some(f) => f,
            None => {
                debug_log("InputPin::Receive dropped: no fmt");
                return Ok(());
            }
        };
        debug_log(&format!(
            "InputPin::Receive ok fmt={}x{} bpp={}",
            fmt.width, fmt.height, fmt.bpp
        ));
        let (width, height, bpp) = (fmt.width, fmt.height, fmt.bpp);

        let mut start = 0i64;
        let mut stop = 0i64;
        let has_time = unsafe { sample.GetTime(&mut start, &mut stop).is_ok() };
        if has_time {
            core.ts_queue.push_back(start);
            core.last_ts = start;
        } else {
            start = core.last_ts + fmt.frame_dur;
            core.ts_queue.push_back(start);
            core.last_ts = start;
        }
        drop(core);

        let bytes_per_pixel = (bpp / 8).max(1) as usize;
        let row = width as usize * bytes_per_pixel;
        let tight = row * height as usize;
        let stride = (row + 3) & !3;
        let mut depad: Vec<u8> = Vec::new();
        let frame: &[u8] = if data.len() == tight {
            data
        } else if data.len() >= stride * height as usize && stride != row {
            // DIB rows are padded to 4 bytes; ffmpeg expects tight rows.
            depad.reserve(tight);
            for y in 0..height as usize {
                depad.extend_from_slice(&data[y * stride..y * stride + row]);
            }
            &depad[..]
        } else if data.len() < tight {
            // Short sample: pad with zeros so the ffmpeg frame boundary
            // stays intact instead of desynchronizing the whole stream.
            depad.resize(tight, 0);
            depad[..data.len()].copy_from_slice(data);
            &depad[..]
        } else {
            // Extra trailing bytes: keep only one exact frame.
            depad.extend_from_slice(&data[..tight]);
            &depad[..]
        };

        {
            let mut enc = self.shared.encoder.lock().unwrap();
            match enc.as_mut() {
                Some(e) => {
                    if let Err(ioe) = e.write_frame(frame) {
                        if !crate::state::report_failure(&format!(
                            "InputPin::Receive write_frame failed: {}",
                            ioe
                        )) {
                            crate::state::debug_log(
                                "InputPin::Receive write_frame failed (already reported)",
                            );
                        }
                        // Encoder is dead; drop this frame and let the graph
                        // finish cleanly (the failure is already shown).
                    }
                }
                None => {
                    // Encoder not started yet: buffer the frame data so the
                    // beginning of the render is not lost.
                    self.shared
                        .core
                        .lock()
                        .unwrap()
                        .pending_frames
                        .push(frame.to_vec());
                }
            }
        }
        drain_deliver(&self.shared);
        Ok(())
    }

    fn ReceiveMultiple(&self, psamples: *const Option<IMediaSample>, nsamples: i32) -> Result<i32> {
        crate::state::always_log("InputPin::ReceiveMultiple");
        if psamples.is_null() || nsamples <= 0 {
            return Ok(0);
        }
        let mut processed = 0;
        for i in 0..nsamples {
            let s = unsafe { &*psamples.add(i as usize) };
            let Some(s) = s else {
                break;
            };
            let opt: Option<IMediaSample> = Some(s.clone());
            self.Receive(Ref::from(&opt))?;
            processed += 1;
        }
        Ok(processed)
    }

    fn ReceiveCanBlock(&self) -> Result<()> {
        crate::state::always_log("InputPin::ReceiveCanBlock");
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Output pin (IPin)
// ---------------------------------------------------------------------------

#[implement(IPin)]
pub struct OutputPin {
    shared: Arc<Shared>,
}

impl OutputPin {
    pub fn new(shared: Arc<Shared>) -> Self {
        OutputPin { shared }
    }
}

impl IPin_Impl for OutputPin_Impl {
    fn Connect(&self, preceivepin: Ref<'_, IPin>, pmt: *const AM_MEDIA_TYPE) -> Result<()> {
        crate::state::always_log(&format!(
            "OutputPin::Connect enter pmt_null={}",
            pmt.is_null()
        ));
        if let Some(peer) = preceivepin.cloned() {
            let mut info = PIN_INFO::default();
            if unsafe { peer.QueryPinInfo(&mut info) }.is_ok() {
                let mut name = String::new();
                for ch in info.achName.iter() {
                    if *ch == 0 {
                        break;
                    }
                    name.push(char::from_u32(*ch as u32).unwrap_or('?'));
                }
                debug_log(&format!(
                    "OutputPin::Connect peer dir={} name={}",
                    info.dir.0, name
                ));
            }
        }
        {
            let conn = self.shared.output.lock().unwrap();
            if conn.peer.is_some() {
                return Err(err(VFW_E_ALREADY_CONNECTED));
            }
        }

        let built = {
            let core = self.shared.core.lock().unwrap();
            let fmt = core.fmt.as_ref().ok_or(err(VFW_E_NOT_CONNECTED))?;
            let fourcc = core.fourcc;
            let mt = mediatype::make_output_type(fmt, fourcc);
            (mt, fmt.width, fmt.height, fmt.bpp, fmt.frame_dur)
        };
        let (mut mt, width, height, bpp, frame_dur) = built;

        if !pmt.is_null() {
            let proposed = unsafe { pmt.as_ref() }.ok_or(err(E_POINTER))?;
            unsafe { mediatype::free_mt(&mut mt) };
            mt = mediatype::clone_mt(proposed);
        }

        let peer = preceivepin.cloned().ok_or(err(E_POINTER))?;

        let self_pin = self
            .shared
            .output_pin
            .lock()
            .unwrap()
            .clone()
            .ok_or(err(E_FAIL))?;
        // Note: AVI Mux returns S_FALSE from QueryAccept for every video
        // type, so the real acceptance check is ReceiveConnection. Do NOT
        // hold our output-pin mutex here: the downstream pin may call back
        // into ConnectedTo/QueryPinInfo while negotiating, and a non-reentrant
        // mutex would deadlock the graph thread.
        let rc = unsafe { peer.ReceiveConnection(&self_pin, &mt) };
        debug_log(&format!("OutputPin::Connect ReceiveConnection hr={:08X}", hr_of(&rc)));
        rc?;

        let meminput: IMemInputPin = match peer.cast() {
            Ok(m) => m,
            Err(e) => {
                debug_log(&format!("OutputPin::Connect cast IMemInputPin failed {:08X}", e.code().0));
                unsafe { mediatype::free_mt(&mut mt) };
                return Err(err(VFW_E_TYPE_NOT_ACCEPTED));
            }
        };

        // The output buffer must fit the largest compressed sample. High
        // CBR targets can produce I-frames far larger than the uncompressed
        // frame (e.g. 85 Mbps at 320x240 ~= 350 KB/frame vs 230 KB raw), so
        // size the allocator from the configured bitrate as well.
        let cfg = crate::config::load();
        let uncompressed = width * height * ((bpp / 8) as i32);
        let fps = if frame_dur > 0 {
            10_000_000.0 / frame_dur as f64
        } else {
            30.0
        };
        let bitrate_frame_bytes = if cfg.bitrate > 0 {
            (((cfg.bitrate as f64 / 8.0) / fps) * 2.0) as i32
        } else {
            0
        };
        let req = ALLOCATOR_PROPERTIES {
            cBuffers: 8,
            cbBuffer: uncompressed.max(bitrate_frame_bytes).max(65536),
            cbAlign: 1,
            cbPrefix: 0,
        };

        let mut own_alloc = false;
        let (allocator, actual_cb) = match unsafe { meminput.GetAllocator() } {
            Ok(a) => {
                debug_log("OutputPin::Connect allocator: downstream");
                let actual = unsafe { a.SetProperties(&req) }?;
                let na = unsafe { meminput.NotifyAllocator(&a, false) };
                debug_log(&format!(
                    "OutputPin::Connect NotifyAllocator(downstream) hr={:08X} actual_cb={}",
                    hr_of(&na),
                    actual.cbBuffer
                ));
                na?;
                (a, actual.cbBuffer)
            }
            Err(e) if e.code() == E_NOTIMPL_HR => {
                debug_log("OutputPin::Connect allocator: own");
                let a: IMemAllocator = MemAllocator::new().into();
                let actual = unsafe { a.SetProperties(&req) }?;
                let na = unsafe { meminput.NotifyAllocator(&a, false) };
                debug_log(&format!(
                    "OutputPin::Connect NotifyAllocator hr={:08X} actual_cb={}",
                    hr_of(&na),
                    actual.cbBuffer
                ));
                na?;
                own_alloc = true;
                (a, actual.cbBuffer)
            }
            Err(e) => {
                debug_log(&format!("OutputPin::Connect GetAllocator failed {:08X}", e.code().0));
                unsafe { mediatype::free_mt(&mut mt) };
                return Err(e);
            }
        };
        debug_log("OutputPin::Connect OK");

        let mut conn = self.shared.output.lock().unwrap();
        if conn.peer.is_some() {
            unsafe { mediatype::free_mt(&mut mt) };
            return Err(err(VFW_E_ALREADY_CONNECTED));
        }
        conn.peer = Some(peer);
        conn.meminput = Some(meminput);
        conn.allocator = Some(allocator);
        conn.own_alloc = own_alloc;
        conn.mt = Some(mt);
        conn.cb_buffer = actual_cb;
        Ok(())
    }

    fn ReceiveConnection(&self, _pconnector: Ref<'_, IPin>, _pmt: *const AM_MEDIA_TYPE) -> Result<()> {
        Err(err(E_UNEXPECTED_HR))
    }

    fn Disconnect(&self) -> Result<()> {
        let mut conn = self.shared.output.lock().unwrap();
        if conn.peer.is_none() {
            return Err(err(S_FALSE_HR));
        }
        conn.clear();
        Ok(())
    }

    fn ConnectedTo(&self) -> Result<IPin> {
        crate::state::always_log("OutputPin::ConnectedTo");
        self.shared
            .output
            .lock()
            .unwrap()
            .peer
            .clone()
            .ok_or(err(VFW_E_NOT_CONNECTED))
    }

    fn ConnectionMediaType(&self, pmt: *mut AM_MEDIA_TYPE) -> Result<()> {
        crate::state::always_log("OutputPin::ConnectionMediaType");
        let conn = self.shared.output.lock().unwrap();
        let mt = conn.mt.as_ref().ok_or(err(VFW_E_NOT_CONNECTED))?;
        if pmt.is_null() {
            return Err(err(E_POINTER));
        }
        unsafe {
            *pmt = mediatype::clone_mt(mt);
        }
        Ok(())
    }

    fn QueryPinInfo(&self, pinfo: *mut PIN_INFO) -> Result<()> {
        crate::state::always_log("OutputPin::QueryPinInfo");
        if pinfo.is_null() {
            return Err(err(E_POINTER));
        }
        let mut info = PIN_INFO {
            dir: PINDIR_OUTPUT,
            ..Default::default()
        };
        fill_ach_name(&mut info.achName, "FFmpegOut");
        info.pFilter = core::mem::ManuallyDrop::new(self.shared.filter_iface());
        unsafe {
            pinfo.write(info);
        }
        Ok(())
    }

    fn QueryDirection(&self) -> Result<PIN_DIRECTION> {
        crate::state::always_log("OutputPin::QueryDirection");
        Ok(PINDIR_OUTPUT)
    }

    fn QueryId(&self) -> Result<PWSTR> {
        crate::state::always_log("OutputPin::QueryId");
        query_id("FFmpegOut")
    }

    fn QueryAccept(&self, pmt: *const AM_MEDIA_TYPE) -> HRESULT {
        crate::state::always_log("OutputPin::QueryAccept");
        let Some(mt) = (unsafe { pmt.as_ref() }) else {
            return E_POINTER;
        };
        let core = self.shared.core.lock().unwrap();
        let expected = mediatype::fourcc_guid(core.fourcc);
        if mt.majortype == MEDIATYPE_Video && mt.subtype == expected {
            HRESULT(0)
        } else {
            VFW_E_TYPE_NOT_ACCEPTED
        }
    }

    fn EnumMediaTypes(&self) -> Result<IEnumMediaTypes> {
        crate::state::always_log("OutputPin::EnumMediaTypes");
        let core = self.shared.core.lock().unwrap();
        match core.fmt.as_ref() {
            Some(fmt) => {
                let mt = mediatype::make_output_type(fmt, core.fourcc);
                Ok(EnumMediaTypes::new(vec![mt]).into())
            }
            None => Ok(EnumMediaTypes::new(Vec::new()).into()),
        }
    }

    fn QueryInternalConnections(&self, appin: OutRef<'_, IPin>, npin: *mut u32) -> Result<()> {
        crate::state::always_log("OutputPin::QueryInternalConnections");
        let _ = (appin, npin);
        Err(err(E_NOTIMPL_HR))
    }

    fn EndOfStream(&self) -> Result<()> {
        Err(err(E_UNEXPECTED_HR))
    }

    fn BeginFlush(&self) -> Result<()> {
        Err(err(E_UNEXPECTED_HR))
    }

    fn EndFlush(&self) -> Result<()> {
        Err(err(E_UNEXPECTED_HR))
    }

    fn NewSegment(&self, _tstart: i64, _tstop: i64, _drate: f64) -> Result<()> {
        Err(err(E_UNEXPECTED_HR))
    }
}
