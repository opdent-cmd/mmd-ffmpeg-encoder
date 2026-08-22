#include "ffmpeg_encoder.h"

#include <algorithm>
#include <cstdlib>
#include <cstring>
#include <cwchar>
#include <utility>
#include <wchar.h>

extern HINSTANCE g_hInst;

namespace {
    const WCHAR kIniName[] = L"FFmpegEncoder.ini";
    const GUID kMmdRgb32 = {
        0x773C9AC0, 0x3274, 0x11D0,
        {0xB7, 0x24, 0x00, 0xAA, 0x00, 0x6C, 0x1A, 0x01}};

    bool FileExists(const std::wstring& path)
    {
        const DWORD attr = GetFileAttributesW(path.c_str());
        return attr != INVALID_FILE_ATTRIBUTES && !(attr & FILE_ATTRIBUTE_DIRECTORY);
    }

    std::wstring ModuleDirectory(HMODULE module)
    {
        std::vector<WCHAR> buf(32768);
        const DWORD n = GetModuleFileNameW(module, buf.data(),
                                           static_cast<DWORD>(buf.size()));
        if (n == 0 || n >= buf.size()) {
            return std::wstring();
        }
        std::wstring path(buf.data(), n);
        const size_t slash = path.find_last_of(L"\\/");
        return slash == std::wstring::npos ? std::wstring() : path.substr(0, slash);
    }

    std::wstring JoinPath(const std::wstring& dir, const std::wstring& name)
    {
        if (dir.empty()) {
            return name;
        }
        return dir + L"\\" + name;
    }

    std::wstring Trim(std::wstring value)
    {
        const size_t first = value.find_first_not_of(L" \t\r\n");
        if (first == std::wstring::npos) {
            return std::wstring();
        }
        const size_t last = value.find_last_not_of(L" \t\r\n");
        return value.substr(first, last - first + 1);
    }

    std::wstring Lower(std::wstring value)
    {
        for (WCHAR& ch : value) {
            if (ch >= L'A' && ch <= L'Z') {
                ch = static_cast<WCHAR>(ch - L'A' + L'a');
            }
        }
        return value;
    }

    bool SafeToken(const std::wstring& value)
    {
        if (value.empty()) {
            return false;
        }
        for (WCHAR ch : value) {
            if (!((ch >= L'a' && ch <= L'z') ||
                  (ch >= L'A' && ch <= L'Z') ||
                  (ch >= L'0' && ch <= L'9') ||
                  ch == L'_' || ch == L'-' || ch == L'.')) {
                return false;
            }
        }
        return true;
    }

    bool SamePath(const std::wstring& left, const std::wstring& right)
    {
        if (left.empty() || right.empty()) {
            return false;
        }
        WCHAR leftBuf[32768] = {};
        WCHAR rightBuf[32768] = {};
        const DWORD leftLen = GetFullPathNameW(left.c_str(), ARRAYSIZE(leftBuf),
                                               leftBuf, NULL);
        const DWORD rightLen = GetFullPathNameW(right.c_str(), ARRAYSIZE(rightBuf),
                                                rightBuf, NULL);
        if (leftLen == 0 || rightLen == 0 ||
            leftLen >= ARRAYSIZE(leftBuf) || rightLen >= ARRAYSIZE(rightBuf)) {
            return _wcsicmp(left.c_str(), right.c_str()) == 0;
        }
        return _wcsicmp(leftBuf, rightBuf) == 0;
    }
}

static void AppendQuoted(std::wstring& s, const std::wstring& arg);

CUnknown* WINAPI CFFmpegEncoder::CreateInstance(LPUNKNOWN pUnk, HRESULT* phr)
{
    return new CFFmpegEncoder(pUnk, phr);
}

CFFmpegEncoder::CFFmpegEncoder(LPUNKNOWN pUnk, HRESULT* phr)
    : CTransformFilter(NAME("FFmpeg Video Encoder"), pUnk, CLSID_FFmpegEncoder)
{
    const HRESULT hr = ReadConfig();
    if (phr && FAILED(hr)) {
        *phr = hr;
    }
}

CFFmpegEncoder::~CFFmpegEncoder()
{
    StopFFmpeg();
}

// ---------------------------------------------------------------------------
// Media type negotiation
// ---------------------------------------------------------------------------

HRESULT CFFmpegEncoder::CheckInputType(const CMediaType* mtIn)
{
    if (!mtIn) {
        return E_POINTER;
    }
    if (mtIn->majortype != MEDIATYPE_Video) {
        return VFW_E_TYPE_NOT_ACCEPTED;
    }
    if (!(IsEqualGUID(*mtIn->Subtype(), MEDIASUBTYPE_RGB24) ||
          IsEqualGUID(*mtIn->Subtype(), MEDIASUBTYPE_RGB32) ||
          IsEqualGUID(*mtIn->Subtype(), MEDIASUBTYPE_ARGB32) ||
          IsEqualGUID(*mtIn->Subtype(), MEDIASUBTYPE_RGB565) ||
          IsEqualGUID(*mtIn->Subtype(), MEDIASUBTYPE_RGB555) ||
          IsEqualGUID(*mtIn->Subtype(), MEDIASUBTYPE_RGB8) ||
          IsEqualGUID(*mtIn->Subtype(), kMmdRgb32))) {
        return VFW_E_TYPE_NOT_ACCEPTED;
    }
    if ((*mtIn->FormatType() == FORMAT_VideoInfo &&
         mtIn->FormatLength() < sizeof(VIDEOINFOHEADER)) ||
        (*mtIn->FormatType() == FORMAT_VideoInfo2 &&
         mtIn->FormatLength() < sizeof(VIDEOINFOHEADER2)) ||
        (*mtIn->FormatType() != FORMAT_VideoInfo &&
         *mtIn->FormatType() != FORMAT_VideoInfo2) || !mtIn->Format()) {
        return VFW_E_TYPE_NOT_ACCEPTED;
    }
    return S_OK;
}

GUID CFFmpegEncoder::FourccGuid(DWORD fcc)
{
    GUID g;
    g.Data1 = fcc;
    g.Data2 = 0x0000;
    g.Data3 = 0x0010;
    g.Data4[0] = 0x80; g.Data4[1] = 0x00; g.Data4[2] = 0x00;
    g.Data4[3] = 0xAA; g.Data4[4] = 0x00; g.Data4[5] = 0x38;
    g.Data4[6] = 0x9B; g.Data4[7] = 0x71;
    return g;
}

HRESULT CFFmpegEncoder::CheckTransform(const CMediaType* mtIn, const CMediaType* mtOut)
{
    if (!mtOut) {
        return E_POINTER;
    }
    HRESULT hr = CheckInputType(mtIn);
    if (FAILED(hr)) {
        return hr;
    }
    if (mtOut->majortype != MEDIATYPE_Video) {
        return VFW_E_TYPE_NOT_ACCEPTED;
    }
    GUID expected = FourccGuid(m_fourcc);
    if (!IsEqualGUID(*mtOut->Subtype(), expected)) {
        return VFW_E_TYPE_NOT_ACCEPTED;
    }
    return S_OK;
}

HRESULT CFFmpegEncoder::GetMediaType(int iPosition, CMediaType* pmt)
{
    if (!pmt) {
        return E_POINTER;
    }
    if (iPosition < 0) {
        return E_INVALIDARG;
    }
    if (iPosition > 0) {
        return VFW_S_NO_MORE_ITEMS;
    }
    if (!m_pInput->IsConnected() || m_width <= 0 || m_height <= 0) {
        return VFW_E_NOT_CONNECTED;
    }

    VIDEOINFOHEADER vih;
    ZeroMemory(&vih, sizeof(vih));
    vih.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    vih.bmiHeader.biWidth = m_width;
    // AVI Mux rejects compressed types with a negative (top-down) height.
    vih.bmiHeader.biHeight = m_height;
    vih.bmiHeader.biPlanes = 1;
    vih.bmiHeader.biBitCount = 24;
    vih.bmiHeader.biCompression = m_fourcc;
    vih.bmiHeader.biSizeImage = 0;        // compressed stream
    vih.AvgTimePerFrame = m_frameDur;
    SetRect(&vih.rcSource, 0, 0, m_width, m_height);
    vih.rcTarget = vih.rcSource;

    GUID sub = FourccGuid(m_fourcc);
    pmt->InitMediaType();
    pmt->SetType(&MEDIATYPE_Video);
    pmt->SetSubtype(&sub);
    pmt->SetFormatType(&FORMAT_VideoInfo);
    pmt->SetTemporalCompression(TRUE);
    pmt->SetSampleSize(0);
    if (!pmt->SetFormat((BYTE*)&vih, sizeof(vih))) {
        return E_OUTOFMEMORY;
    }
    return S_OK;
}

HRESULT CFFmpegEncoder::SetMediaType(PIN_DIRECTION direction, const CMediaType* pmt)
{
    if (!pmt) {
        return E_POINTER;
    }
    if (direction == PINDIR_INPUT) {
        const HRESULT check = CheckInputType(pmt);
        if (FAILED(check)) {
            return check;
        }
        if (*pmt->FormatType() == FORMAT_VideoInfo) {
            VIDEOINFOHEADER* vih = (VIDEOINFOHEADER*)pmt->Format();
            m_width = vih->bmiHeader.biWidth;
            if (vih->bmiHeader.biHeight == LONG_MIN) {
                return VFW_E_INVALIDMEDIATYPE;
            }
            m_height = abs(vih->bmiHeader.biHeight);
            m_bottomUp = vih->bmiHeader.biHeight > 0;
            m_frameDur = vih->AvgTimePerFrame > 0 ? vih->AvgTimePerFrame : m_frameDur;
        } else if (*pmt->FormatType() == FORMAT_VideoInfo2) {
            VIDEOINFOHEADER2* vih2 = (VIDEOINFOHEADER2*)pmt->Format();
            m_width = vih2->bmiHeader.biWidth;
            if (vih2->bmiHeader.biHeight == LONG_MIN) {
                return VFW_E_INVALIDMEDIATYPE;
            }
            m_height = abs(vih2->bmiHeader.biHeight);
            m_bottomUp = vih2->bmiHeader.biHeight > 0;
            m_frameDur = vih2->AvgTimePerFrame > 0 ? vih2->AvgTimePerFrame : m_frameDur;
        } else {
            return VFW_E_INVALIDMEDIATYPE;
        }

        if (m_width <= 0 || m_height <= 0 || m_width > 16384 || m_height > 16384) {
            return VFW_E_INVALIDMEDIATYPE;
        }
        m_inSubtype = *pmt->Subtype();
        if (IsEqualGUID(m_inSubtype, MEDIASUBTYPE_RGB24)) {
            // DirectShow RGB24 samples use DIB byte order: B, G, R.
            m_pixfmt = L"bgr24";  m_bpp = 24;
        } else if (IsEqualGUID(m_inSubtype, MEDIASUBTYPE_RGB32) ||
                   IsEqualGUID(m_inSubtype, MEDIASUBTYPE_ARGB32) ||
                   IsEqualGUID(m_inSubtype, kMmdRgb32)) {
            m_pixfmt = L"bgra";   m_bpp = 32;
        } else if (IsEqualGUID(m_inSubtype, MEDIASUBTYPE_RGB565)) {
            m_pixfmt = L"rgb565le"; m_bpp = 16;
        } else if (IsEqualGUID(m_inSubtype, MEDIASUBTYPE_RGB555)) {
            m_pixfmt = L"rgb555le"; m_bpp = 16;
        } else if (IsEqualGUID(m_inSubtype, MEDIASUBTYPE_RGB8)) {
            m_pixfmt = L"gray";   m_bpp = 8;
        } else {
            return VFW_E_INVALIDMEDIATYPE;
        }
    }
    return CTransformFilter::SetMediaType(direction, pmt);
}

HRESULT CFFmpegEncoder::DecideBufferSize(IMemAllocator* pAlloc,
                                         ALLOCATOR_PROPERTIES* pProps)
{
    if (!pAlloc || !pProps) {
        return E_POINTER;
    }
    // Compressed frames are typically far smaller than the raw input frame,
    // so size the allocator from the input frame size with a sane minimum.
    long cb = 65536;
    if (m_pInput->IsConnected()) {
        long sampleSize = m_pInput->CurrentMediaType().GetSampleSize();
        if (sampleSize > 0) {
            cb = sampleSize;
        }
    }
    if (m_width > 0 && m_height > 0 && m_bpp > 0) {
        const LONGLONG raw = static_cast<LONGLONG>(m_width) * m_height * (m_bpp / 8);
        if (raw > cb && raw <= LONG_MAX) {
            cb = static_cast<long>(raw);
        }
    }
    pProps->cbBuffer = cb;
    pProps->cBuffers = 1;
    pProps->cbAlign = 1;
    pProps->cbPrefix = 0;

    ALLOCATOR_PROPERTIES actual;
    HRESULT hr = pAlloc->SetProperties(pProps, &actual);
    if (FAILED(hr)) {
        return hr;
    }
    if (actual.cbBuffer < pProps->cbBuffer) {
        return E_FAIL;
    }
    return S_OK;
}

// ---------------------------------------------------------------------------
// Streaming lifecycle
// ---------------------------------------------------------------------------

HRESULT CFFmpegEncoder::StartStreaming()
{
    StopFFmpeg();

    CAutoLock lock(&m_lock);
    m_pktQueue.clear();
    m_tsQueue.clear();
    m_inBuf.clear();
    m_group.clear();
    m_groupHasVcl = false;
    m_lastTs = 0;
    m_firstPkt = true;
    m_readerDone = false;
    m_completed = false;
    ResolveOutputPaths();
    const HRESULT hr = StartFFmpeg();
    m_started = SUCCEEDED(hr);
    return hr;
}

HRESULT CFFmpegEncoder::StopStreaming()
{
    StopFFmpeg();
    PostProcessOutputs();
    m_started = false;
    return S_OK;
}

// ---------------------------------------------------------------------------
// ffmpeg child process
// ---------------------------------------------------------------------------

HRESULT CFFmpegEncoder::ReadConfig()
{
    const std::wstring dllDir = ModuleDirectory(g_hInst);
    const std::wstring hostDir = ModuleDirectory(NULL);
    std::wstring ini = JoinPath(dllDir, kIniName);
    if (!FileExists(ini)) {
        ini = JoinPath(hostDir, kIniName);
    }

    WCHAR buf[32768];
    GetPrivateProfileStringW(L"ffmpeg", L"path", L"ffmpeg", buf,
                             ARRAYSIZE(buf), ini.c_str());
    m_ffmpegPath = Trim(buf);
    if (m_ffmpegPath.empty()) {
        m_ffmpegPath = L"ffmpeg";
    }
    if (m_ffmpegPath != L"ffmpeg" && !FileExists(m_ffmpegPath)) {
        const std::wstring installed = JoinPath(JoinPath(dllDir, L"bin"), L"ffmpeg.exe");
        const std::wstring besideDll = JoinPath(dllDir, L"ffmpeg.exe");
        m_ffmpegPath = FileExists(installed) ? installed
                     : FileExists(besideDll) ? besideDll : L"ffmpeg";
    }
    GetPrivateProfileStringW(L"video", L"codec", L"libx264", buf,
                             ARRAYSIZE(buf), ini.c_str());
    m_codec = Lower(Trim(buf));
    if (!SafeToken(m_codec)) {
        m_codec = L"libx264";
    }
    if (_wcsnicmp(m_codec.c_str(), L"auto", 4) == 0) {
        // The most portable automatic path is the bundled software encoder.
        // Hardware encoders remain available when selected explicitly.
        if (m_codec.find(L"265") != std::wstring::npos ||
            m_codec.find(L"hevc") != std::wstring::npos) {
            m_codec = L"libx265";
        } else if (m_codec.find(L"av1") != std::wstring::npos) {
            m_codec = L"libaom-av1";
        } else {
            m_codec = L"libx264";
        }
    }
    GetPrivateProfileStringW(L"video", L"preset", L"veryfast", buf,
                             ARRAYSIZE(buf), ini.c_str());
    m_preset = Trim(buf);
    if (!SafeToken(m_preset)) {
        m_preset = L"veryfast";
    }
    GetPrivateProfileStringW(L"video", L"extra", L"", buf,
                             ARRAYSIZE(buf), ini.c_str());
    m_extra = Trim(buf);
    GetPrivateProfileStringW(L"video", L"container", L"", buf,
                             ARRAYSIZE(buf), ini.c_str());
    m_container = Lower(Trim(buf));
    if (m_container != L"" && m_container != L"mp4" &&
        m_container != L"m4v" && m_container != L"mkv" &&
        m_container != L"matroska" && m_container != L"mov" &&
        m_container != L"webm") {
        m_container.clear();
    }
    GetPrivateProfileStringW(L"video", L"container_path", L"", buf,
                             ARRAYSIZE(buf), ini.c_str());
    m_containerPath = Trim(buf);
    GetPrivateProfileStringW(L"video", L"alpha_format", L"", buf,
                             ARRAYSIZE(buf), ini.c_str());
    m_alphaFormat = Lower(Trim(buf));
    if (m_alphaFormat != L"" && m_alphaFormat != L"webm_vp9" &&
        m_alphaFormat != L"vp9" && m_alphaFormat != L"webm_av1" &&
        m_alphaFormat != L"av1" && m_alphaFormat != L"mov_prores" &&
        m_alphaFormat != L"prores" && m_alphaFormat != L"mkv_ffv1" &&
        m_alphaFormat != L"ffv1") {
        m_alphaFormat.clear();
    }
    m_crf = GetPrivateProfileIntW(L"video", L"crf", 18, ini.c_str());
    m_bitrate = GetPrivateProfileIntW(L"video", L"bitrate", 0, ini.c_str());
    m_crf = max(0, min(m_crf, 51));
    m_bitrate = max(0L, m_bitrate);
    GetPrivateProfileStringW(L"video", L"rate_mode", L"crf", buf,
                             ARRAYSIZE(buf), ini.c_str());
    m_cbr = Lower(Trim(buf)) == L"cbr" && m_bitrate > 0;
    m_deleteAvi = GetPrivateProfileIntW(L"video", L"delete_avi", 1,
                                        ini.c_str()) != 0;
    m_mergeAudio = GetPrivateProfileIntW(L"video", L"merge_audio", 1,
                                         ini.c_str()) != 0;

    m_nvenc = (m_codec.find(L"nvenc") != std::wstring::npos);
    m_qsv = (m_codec.find(L"qsv") != std::wstring::npos);
    m_amf = (m_codec.find(L"amf") != std::wstring::npos);
    m_lossless = (m_codec.find(L"ffv1") != std::wstring::npos ||
                  m_codec.find(L"utvideo") != std::wstring::npos ||
                  m_codec.find(L"huffyuv") != std::wstring::npos ||
                  m_codec.find(L"ffvhuff") != std::wstring::npos);

    // AVI Mux receives Annex B H.264/HEVC. Codecs with another elementary
    // stream syntax are written to the requested side container while the
    // temporary MMD AVI gets a universally parseable H.264 stream.
    m_aviCodec = m_codec;
    if (!m_alphaFormat.empty() ||
        (m_codec.find(L"264") == std::wstring::npos &&
         m_codec.find(L"265") == std::wstring::npos &&
         m_codec.find(L"hevc") == std::wstring::npos)) {
        m_aviCodec = L"libx264";
    }

    if (m_aviCodec.find(L"264") != std::wstring::npos) {
        m_fourcc = mmioFOURCC('H', '2', '6', '4');
        m_outMux = L"h264";
    } else if (m_aviCodec.find(L"265") != std::wstring::npos ||
               m_aviCodec.find(L"hevc") != std::wstring::npos) {
        m_fourcc = mmioFOURCC('H', 'E', 'V', 'C');
        m_outMux = L"hevc";
    } else {
        m_fourcc = mmioFOURCC('H', '2', '6', '4');
        m_outMux = L"h264";
    }
    return S_OK;
}

void CFFmpegEncoder::ResolveOutputPaths()
{
    m_aviPath.clear();
    m_extraPath.clear();
    if (m_pGraph) {
        IEnumFilters* enumerator = NULL;
        if (SUCCEEDED(m_pGraph->EnumFilters(&enumerator)) && enumerator) {
            IBaseFilter* filter = NULL;
            while (enumerator->Next(1, &filter, NULL) == S_OK) {
                IFileSinkFilter* sink = NULL;
                if (SUCCEEDED(filter->QueryInterface(IID_IFileSinkFilter,
                                                     reinterpret_cast<void**>(&sink))) &&
                    sink) {
                    LPOLESTR path = NULL;
                    if (SUCCEEDED(sink->GetCurFile(&path, NULL)) && path) {
                        m_aviPath = path;
                        CoTaskMemFree(path);
                        sink->Release();
                        filter->Release();
                        break;
                    }
                    sink->Release();
                }
                filter->Release();
                filter = NULL;
            }
            enumerator->Release();
        }
    }

    if (m_container.empty() && m_alphaFormat.empty()) {
        return;
    }
    if (!m_containerPath.empty()) {
        m_extraPath = m_containerPath;
        return;
    }
    if (m_aviPath.empty()) {
        return;
    }

    std::wstring extension = m_container;
    if (!m_alphaFormat.empty()) {
        if (m_alphaFormat.find(L"vp9") != std::wstring::npos ||
            m_alphaFormat.find(L"av1") != std::wstring::npos) {
            extension = L"webm";
        } else if (m_alphaFormat.find(L"prores") != std::wstring::npos) {
            extension = L"mov";
        } else {
            extension = L"mkv";
        }
    }
    if (extension == L"matroska") extension = L"mkv";
    const size_t slash = m_aviPath.find_last_of(L"\\/");
    const size_t dot = m_aviPath.find_last_of(L'.');
    const size_t cut = dot != std::wstring::npos &&
                       (slash == std::wstring::npos || dot > slash)
                         ? dot : m_aviPath.size();
    m_extraPath = m_aviPath.substr(0, cut) + L"." + extension;
}

bool CFFmpegEncoder::RunCommand(const std::wstring& command, DWORD timeoutMs)
{
    if (command.empty()) {
        return false;
    }
    std::wstring mutableCommand = command;
    STARTUPINFOW si = {};
    si.cb = sizeof(si);
    PROCESS_INFORMATION pi = {};
    const WCHAR* application = m_ffmpegPath == L"ffmpeg" ? NULL : m_ffmpegPath.c_str();
    if (!CreateProcessW(application, mutableCommand.data(), NULL, NULL, FALSE,
                        CREATE_NO_WINDOW, NULL, NULL, &si, &pi)) {
        return false;
    }
    CloseHandle(pi.hThread);
    const DWORD wait = WaitForSingleObject(pi.hProcess, timeoutMs);
    if (wait != WAIT_OBJECT_0) {
        TerminateProcess(pi.hProcess, 1);
        WaitForSingleObject(pi.hProcess, 2000);
        CloseHandle(pi.hProcess);
        return false;
    }
    DWORD code = 1;
    GetExitCodeProcess(pi.hProcess, &code);
    CloseHandle(pi.hProcess);
    return code == 0;
}

bool CFFmpegEncoder::MergeAudio()
{
    if (m_aviPath.empty() || m_extraPath.empty() ||
        SamePath(m_aviPath, m_extraPath) ||
        !FileExists(m_aviPath) || !FileExists(m_extraPath)) {
        return false;
    }
    const size_t dot = m_extraPath.find_last_of(L'.');
    const std::wstring temp = dot == std::wstring::npos
        ? m_extraPath + L".merge.mp4"
        : m_extraPath.substr(0, dot) + L".merge" + m_extraPath.substr(dot);
    const std::wstring ext = dot == std::wstring::npos
        ? L"" : Lower(m_extraPath.substr(dot + 1));

    std::wstring cmd;
    AppendQuoted(cmd, m_ffmpegPath);
    cmd += L" -y -hide_banner -loglevel error -nostdin -i";
    AppendQuoted(cmd, m_extraPath);
    cmd += L" -i";
    AppendQuoted(cmd, m_aviPath);
    cmd += L" -map 0:v:0 -map 1:a:0? -c:v copy -c:a ";
    cmd += ext == L"webm" ? L"libopus" : L"aac";
    cmd += L" -b:a 192k -shortest";
    AppendQuoted(cmd, temp);
    if (!RunCommand(cmd, 120000) || !FileExists(temp)) {
        DeleteFileW(temp.c_str());
        return false;
    }
    if (!MoveFileExW(temp.c_str(), m_extraPath.c_str(),
                     MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
        DeleteFileW(temp.c_str());
        return false;
    }
    return true;
}

void CFFmpegEncoder::PostProcessOutputs()
{
    if (!m_completed || m_extraPath.empty() || !FileExists(m_extraPath)) {
        return;
    }
    if (m_mergeAudio && !MergeAudio()) {
        return;
    }
    if (m_deleteAvi && !m_aviPath.empty()) {
        DeleteFileW(m_aviPath.c_str());
    }
}

static void AppendQuoted(std::wstring& s, const std::wstring& arg)
{
    if (!s.empty()) {
        s += L' ';
    }
    if (arg.find(L' ') != std::wstring::npos || arg.empty()) {
        s += L'"';
        s += arg;
        s += L'"';
    } else {
        s += arg;
    }
}

static std::wstring TeeEscape(std::wstring path)
{
    std::replace(path.begin(), path.end(), L'\\', L'/');
    std::wstring out;
    for (WCHAR ch : path) {
        if (ch == L'[' || ch == L']' || ch == L'|' || ch == L':' || ch == L'\'') {
            out += L'\\';
        }
        out += ch;
    }
    return out;
}

void CFFmpegEncoder::AppendCodecArgs(std::wstring& cmd,
                                     const std::wstring& codec,
                                     const std::wstring& preset,
                                     bool bottomUp)
{
    const bool nvenc = codec.find(L"nvenc") != std::wstring::npos;
    const bool qsv = codec.find(L"qsv") != std::wstring::npos;
    const bool amf = codec.find(L"amf") != std::wstring::npos;
    const bool gpu = nvenc || qsv || amf;
    const bool cbr = m_cbr && m_bitrate > 0;
    const bool lossless = codec.find(L"ffv1") != std::wstring::npos ||
                          codec.find(L"utvideo") != std::wstring::npos ||
                          codec.find(L"huffyuv") != std::wstring::npos ||
                          codec.find(L"ffvhuff") != std::wstring::npos;

    if (bottomUp) {
        cmd += qsv ? L" -vf vflip,format=nv12,hwupload=extra_hw_frames=64"
                   : L" -vf vflip";
    } else if (qsv) {
        cmd += L" -vf format=nv12,hwupload=extra_hw_frames=64";
    }
    cmd += L" -c:v ";
    cmd += codec;
    if (!preset.empty() && (!nvenc || preset != L"veryfast")) {
        cmd += amf ? L" -quality " : L" -preset ";
        cmd += preset;
    }
    WCHAR numBuf[64];
    if (gpu) {
        if (m_bitrate > 0) {
            swprintf_s(numBuf, L" -b:v %ld", m_bitrate);
            cmd += numBuf;
            if (cbr) {
                swprintf_s(numBuf, L" -minrate %ld -maxrate %ld -bufsize %ld",
                           m_bitrate, m_bitrate, m_bitrate * 2L);
                cmd += numBuf;
                if (nvenc) {
                    cmd += L" -rc cbr -cbr 1 -strict_gop 1";
                    if (m_nvencCbrPadding) {
                        cmd += L" -cbr_padding 1";
                    }
                } else if (amf) {
                    cmd += L" -rc cbr -enforce_hrd 1 -filler_data 1";
                } else if (qsv) {
                    cmd += L" -low_delay_brc 1";
                }
            }
        } else if (qsv) {
            cmd += L" -global_quality 23";
        } else if (amf) {
            cmd += L" -qp_i 23 -qp_p 23";
        } else {
            cmd += L" -qp 23";
        }
    } else if (m_bitrate > 0) {
        swprintf_s(numBuf, L" -b:v %ld", m_bitrate);
        cmd += numBuf;
        if (cbr) {
            swprintf_s(numBuf, L" -minrate %ld -maxrate %ld -bufsize %ld",
                       m_bitrate, m_bitrate, m_bitrate * 2L);
            cmd += numBuf;
            if (codec.find(L"264") != std::wstring::npos) {
                cmd += L" -x264-params nal-hrd=cbr:filler=1";
            } else if (codec.find(L"265") != std::wstring::npos ||
                       codec.find(L"hevc") != std::wstring::npos) {
                const long kbps = m_bitrate / 1000L;
                const long bufKbps = (m_bitrate * 2L) / 1000L;
                cmd += L" -x265-params bitrate=" + std::to_wstring(kbps) +
                       L":vbv-maxrate=" + std::to_wstring(kbps) +
                       L":vbv-bufsize=" + std::to_wstring(bufKbps);
            }
        }
    } else if (m_crf > 0 && !lossless) {
        swprintf_s(numBuf, L" -crf %d", m_crf);
        cmd += numBuf;
    }
    if (!lossless && !qsv) {
        cmd += L" -pix_fmt yuv420p";
    }
    if (!lossless && !gpu &&
        (codec.find(L"264") != std::wstring::npos ||
         codec.find(L"265") != std::wstring::npos ||
         codec.find(L"hevc") != std::wstring::npos)) {
        cmd += L" -bf 0";
    }
    if (codec == m_codec && !m_extra.empty()) {
        cmd += L" ";
        cmd += m_extra;
    }
}

void CFFmpegEncoder::BuildCmdLine(std::wstring& cmd)
{
    wchar_t fpsBuf[32];
    double fps = 10000000.0 / (double)m_frameDur;
    swprintf_s(fpsBuf, L"%g", fps);

    cmd.clear();
    AppendQuoted(cmd, m_ffmpegPath);
    cmd += L" -y -hide_banner -loglevel error -nostdin";
    if (m_codec.find(L"qsv") != std::wstring::npos) {
        cmd += L" -init_hw_device qsv=hw -filter_hw_device hw";
    }
    cmd += L" -f rawvideo -pix_fmt ";
    cmd += m_pixfmt;
    wchar_t sizeBuf[64];
    swprintf_s(sizeBuf, L" -s %dx%d", m_width, m_height);
    cmd += sizeBuf;
    cmd += L" -r ";
    cmd += fpsBuf;
    cmd += L" -i pipe:0";

    if (!m_alphaFormat.empty() && !m_extraPath.empty()) {
        cmd += L" -map 0:v";
        if (m_bottomUp) {
            cmd += L" -vf vflip";
        }
        if (m_alphaFormat == L"webm_vp9" || m_alphaFormat == L"vp9") {
            cmd += L" -c:v libvpx-vp9 -pix_fmt yuva420p -deadline good -cpu-used 2 -row-mt 1";
        } else if (m_alphaFormat == L"mov_prores" || m_alphaFormat == L"prores") {
            cmd += L" -c:v prores_ks -pix_fmt yuva444p10le -profile:v 4444 -vendor apl0";
        } else {
            cmd += L" -c:v ffv1 -pix_fmt yuva444p -level 3";
        }
        if (m_bitrate > 0) {
            cmd += L" -b:v " + std::to_wstring(m_bitrate);
        } else if (m_crf > 0 && m_alphaFormat.find(L"vp9") != std::wstring::npos) {
            cmd += L" -crf " + std::to_wstring(m_crf);
        }
        if (!m_extra.empty()) {
            cmd += L" " + m_extra;
        }
        AppendQuoted(cmd, m_extraPath);

        cmd += L" -map 0:v";
        AppendCodecArgs(cmd, L"libx264", L"ultrafast", m_bottomUp);
        cmd += L" -f h264 pipe:1";
        return;
    }

    if (m_codec != m_aviCodec && !m_extraPath.empty()) {
        cmd += L" -map 0:v";
        AppendCodecArgs(cmd, m_codec, m_preset, m_bottomUp);
        AppendQuoted(cmd, m_extraPath);
        cmd += L" -map 0:v";
        AppendCodecArgs(cmd, m_aviCodec, L"ultrafast", m_bottomUp);
        cmd += L" -f " + m_outMux + L" pipe:1";
        return;
    }

    cmd += L" -map 0:v";
    AppendCodecArgs(cmd, m_aviCodec, m_preset, m_bottomUp);
    if (!m_extraPath.empty()) {
        std::wstring format = m_container;
        if (format == L"mkv") format = L"matroska";
        if (format == L"m4v") format = L"mp4";
        const std::wstring tee = L"[f=" + format + L":onfail=ignore]" +
                                 TeeEscape(m_extraPath) + L"|[f=" + m_outMux +
                                 L"]pipe\\:1";
        cmd += L" -f tee";
        AppendQuoted(cmd, tee);
    } else {
        cmd += L" -f " + m_outMux + L" pipe:1";
    }
}

HRESULT CFFmpegEncoder::StartFFmpeg()
{
    SelectPortableCodec();

    // Resolve ffmpeg path: if it is a bare name, let CreateProcess search PATH.
    std::wstring cmd;
    BuildCmdLine(cmd);
    if (cmd.empty()) {
        return E_FAIL;
    }

    SECURITY_ATTRIBUTES sa;
    sa.nLength = sizeof(sa);
    sa.bInheritHandle = TRUE;
    sa.lpSecurityDescriptor = NULL;

    HANDLE hStdinRd = NULL, hStdinWr = NULL;
    HANDLE hStdoutRd = NULL, hStdoutWr = NULL;
    if (!CreatePipe(&hStdinRd, &hStdinWr, &sa, 0)) {
        return HRESULT_FROM_WIN32(GetLastError());
    }
    if (!CreatePipe(&hStdoutRd, &hStdoutWr, &sa, 0)) {
        const HRESULT hr = HRESULT_FROM_WIN32(GetLastError());
        CloseHandle(hStdinRd);
        CloseHandle(hStdinWr);
        return hr;
    }
    if (!SetHandleInformation(hStdinWr, HANDLE_FLAG_INHERIT, 0) ||
        !SetHandleInformation(hStdoutRd, HANDLE_FLAG_INHERIT, 0)) {
        const HRESULT hr = HRESULT_FROM_WIN32(GetLastError());
        CloseHandle(hStdinRd);
        CloseHandle(hStdinWr);
        CloseHandle(hStdoutRd);
        CloseHandle(hStdoutWr);
        return hr;
    }

    WCHAR tempDir[32768] = {};
    std::wstring logPath;
    const DWORD tempLen = GetTempPathW(ARRAYSIZE(tempDir), tempDir);
    if (tempLen > 0 && tempLen < ARRAYSIZE(tempDir)) {
        logPath.assign(tempDir, tempLen);
    }
    logPath += L"MMDFfmpegEncoder";
    CreateDirectoryW(logPath.c_str(), NULL);
    logPath += L"\\ffmpeg_encoder_debug.log";
    HANDLE hLog = CreateFileW(logPath.c_str(), FILE_APPEND_DATA,
                              FILE_SHARE_READ | FILE_SHARE_WRITE,
                              &sa, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (hLog == INVALID_HANDLE_VALUE) {
        hLog = CreateFileW(L"NUL", GENERIC_WRITE,
                           FILE_SHARE_READ | FILE_SHARE_WRITE,
                           &sa, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
        if (hLog == INVALID_HANDLE_VALUE) {
            hLog = NULL;
        }
    }

    STARTUPINFOW si;
    ZeroMemory(&si, sizeof(si));
    si.cb = sizeof(si);
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdInput = hStdinRd;
    si.hStdOutput = hStdoutWr;
    si.hStdError = hLog;

    PROCESS_INFORMATION pi;
    ZeroMemory(&pi, sizeof(pi));

    const WCHAR* application = m_ffmpegPath == L"ffmpeg" ? NULL : m_ffmpegPath.c_str();
    BOOL ok = CreateProcessW(application, cmd.data(), NULL, NULL, TRUE,
                             CREATE_NO_WINDOW, NULL, NULL, &si, &pi);

    CloseHandle(hStdinRd);
    CloseHandle(hStdoutWr);
    if (hLog) {
        CloseHandle(hLog);
    }

    if (!ok) {
        const HRESULT hr = HRESULT_FROM_WIN32(GetLastError());
        CloseHandle(hStdinWr);
        CloseHandle(hStdoutRd);
        return hr;
    }

    m_hProc = pi.hProcess;
    m_hChildStdinW = hStdinWr;
    m_hChildStdoutR = hStdoutRd;
    CloseHandle(pi.hThread);

    m_hThread = CreateThread(NULL, 0, ReaderThreadProc, this, 0, NULL);
    if (!m_hThread) {
        StopFFmpeg();
        return E_FAIL;
    }
    return S_OK;
}

void CFFmpegEncoder::SelectPortableCodec()
{
    m_nvencCbrPadding = false;
    const bool hardware = m_codec.find(L"nvenc") != std::wstring::npos ||
                          m_codec.find(L"qsv") != std::wstring::npos ||
                          m_codec.find(L"amf") != std::wstring::npos;
    if (!hardware) {
        return;
    }

    std::wstring probe;
    AppendQuoted(probe, m_ffmpegPath);
    probe += L" -hide_banner -loglevel error -nostdin";
    if (m_codec.find(L"qsv") != std::wstring::npos) {
        probe += L" -init_hw_device qsv=hw -filter_hw_device hw";
    }
    probe += L" -f lavfi -i color=black:s=160x120:r=30:d=0.1 -frames:v 1";
    // Probe the exact hardware rate-control path used by the real render.
    // A minimal '-c:v h264_nvenc' probe can pass while CBR/HRD options fail
    // on an older driver, which used to leave MMD with a dead child process.
    AppendCodecArgs(probe, m_codec, m_preset, false);
    probe += L" -f null -";
    if (RunCommand(probe, 20000)) {
        // FFmpeg 8+ can ask NVENC to insert filler NAL units. This is the
        // only way to keep a flat scene at the requested CBR bitrate. Probe
        // the exact same command with the optional flag so FFmpeg 7.x and
        // older vendor builds remain usable.
        if (m_nvenc && m_cbr) {
            const bool oldPadding = m_nvencCbrPadding;
            m_nvencCbrPadding = true;
            std::wstring paddingProbe;
            AppendQuoted(paddingProbe, m_ffmpegPath);
            paddingProbe += L" -hide_banner -loglevel error -nostdin";
            paddingProbe += L" -f lavfi -i color=black:s=160x120:r=30:d=0.1 -frames:v 1";
            AppendCodecArgs(paddingProbe, m_codec, m_preset, false);
            paddingProbe += L" -f null -";
            if (!RunCommand(paddingProbe, 20000)) {
                m_nvencCbrPadding = oldPadding;
                OutputDebugStringW(L"MMD FFmpeg Encoder: NVENC cbr_padding unsupported; using driver CBR without filler\n");
            } else {
                OutputDebugStringW(L"MMD FFmpeg Encoder: NVENC cbr_padding enabled for strict CBR\n");
            }
        }
        return;
    }

    const std::wstring oldCodec = m_codec;
    if (m_codec.find(L"265") != std::wstring::npos ||
        m_codec.find(L"hevc") != std::wstring::npos) {
        m_codec = L"libx265";
    } else if (m_codec.find(L"av1") != std::wstring::npos) {
        m_codec = L"libaom-av1";
    } else {
        m_codec = L"libx264";
    }
    if (m_aviCodec == oldCodec) {
        m_aviCodec = m_codec;
    }
    m_preset = L"veryfast";
    m_nvenc = m_qsv = m_amf = false;
    m_nvencCbrPadding = false;
}

void CFFmpegEncoder::StopFFmpeg()
{
    if (m_hChildStdinW) {
        CloseHandle(m_hChildStdinW);
        m_hChildStdinW = NULL;
    }
    if (m_hProc) {
        if (WaitForSingleObject(m_hProc, 10000) != WAIT_OBJECT_0) {
            TerminateProcess(m_hProc, 1);
            WaitForSingleObject(m_hProc, 2000);
        }
    }
    if (m_hThread) {
        DWORD wait = WaitForSingleObject(m_hThread, 5000);
        if (wait != WAIT_OBJECT_0) {
            // The reader is doing a synchronous ReadFile on the pipe. Cancel
            // that I/O before waiting again; never close the thread handle
            // while it may still be executing against this filter object.
            CancelSynchronousIo(m_hThread);
            if (m_hChildStdoutR) {
                CloseHandle(m_hChildStdoutR);
                m_hChildStdoutR = NULL;
            }
            wait = WaitForSingleObject(m_hThread, 5000);
        }
        if (wait != WAIT_OBJECT_0) {
            // A live reader owns a pointer to this object. Waiting here is
            // preferable to returning with a potential use-after-free.
            WaitForSingleObject(m_hThread, INFINITE);
        }
        CloseHandle(m_hThread);
        m_hThread = NULL;
    }
    if (m_hChildStdoutR) {
        CloseHandle(m_hChildStdoutR);
        m_hChildStdoutR = NULL;
    }
    if (m_hProc) {
        CloseHandle(m_hProc);
        m_hProc = NULL;
    }
}

// ---------------------------------------------------------------------------
// Receive / frame feeding
// ---------------------------------------------------------------------------

HRESULT CFFmpegEncoder::Receive(IMediaSample* pSample)
{
    if (!pSample) {
        return E_POINTER;
    }
    AM_SAMPLE2_PROPERTIES* pProps = m_pInput->SampleProps();
    if (pProps->dwStreamId != AM_STREAM_MEDIA) {
        return m_pOutput->Deliver(pSample);
    }
    if (!m_started || !m_hProc || !m_hChildStdinW) {
        return S_OK;
    }

    BYTE* pSrc = NULL;
    long cb = pSample->GetActualDataLength();
    if (FAILED(pSample->GetPointer(&pSrc)) || !pSrc || cb <= 0) {
        return S_OK;
    }

    REFERENCE_TIME tStart = 0, tStop = 0;
    if (FAILED(pSample->GetTime(&tStart, &tStop))) {
        CAutoLock lock(&m_lock);
        tStart = m_lastTs + m_frameDur;
        m_lastTs = tStart;
    }
    {
        CAutoLock lock(&m_lock);
        m_tsQueue.push_back(tStart);
        m_lastTs = tStart;
    }

    HRESULT hr = WriteFrame(pSrc, cb);
    if (FAILED(hr)) {
        return hr;
    }
    return DrainQueue();
}

HRESULT CFFmpegEncoder::WriteFrame(const BYTE* data, long cb)
{
    if (m_width <= 0 || m_height <= 0 || m_bpp <= 0) {
        return VFW_E_WRONG_STATE;
    }
    const int bytesPerPixel = m_bpp / 8;
    const long row = static_cast<long>(m_width) * bytesPerPixel;
    const long stride = (row + 3) & ~3L;
    const long tight = row * m_height;
    const long padded = stride * m_height;

    const BYTE* src = data;
    long len = cb;
    std::vector<BYTE> depad;

    if (cb < tight) {
        return VFW_E_BUFFER_UNDERFLOW;
    }
    if (stride != row && cb >= padded) {
        // DIB rows are DWORD aligned; strip padding for ffmpeg rawvideo.
        depad.resize(tight);
        const BYTE* p = data;
        for (int y = 0; y < m_height; y++) {
            memcpy(&depad[static_cast<size_t>(y) * row], p, row);
            p += stride;
        }
        src = depad.data();
    }
    len = tight;

    DWORD written = 0;
    const BYTE* p = src;
    long remaining = len;
    while (remaining > 0) {
        if (!WriteFile(m_hChildStdinW, p, (DWORD)remaining, &written, NULL)) {
            return HRESULT_FROM_WIN32(GetLastError());
        }
        if (written == 0) {
            return E_FAIL;
        }
        p += written;
        remaining -= (long)written;
    }
    return S_OK;
}

HRESULT CFFmpegEncoder::DrainQueue()
{
    for (;;) {
        Packet pkt;
        {
            CAutoLock lock(&m_lock);
            if (m_pktQueue.empty()) {
                return S_OK;
            }
            pkt = std::move(m_pktQueue.front());
            m_pktQueue.pop_front();
        }
        const HRESULT hr = DeliverPacket(pkt);
        if (FAILED(hr)) {
            return hr;
        }
    }
}

HRESULT CFFmpegEncoder::DeliverPacket(Packet& pkt)
{
    if (pkt.data.empty() || !m_pOutput->IsConnected()) {
        return S_OK;
    }

    REFERENCE_TIME start, stop;
    {
        CAutoLock lock(&m_lock);
        if (!m_tsQueue.empty()) {
            start = m_tsQueue.front();
            m_tsQueue.pop_front();
        } else {
            start = m_lastTs + m_frameDur;
        }
        m_lastTs = start;
    }
    stop = start + m_frameDur;

    IMediaSample* pOut = NULL;
    DWORD flags = m_firstPkt ? 0 : AM_GBF_NOTASYNCPOINT;
    HRESULT hr = m_pOutput->GetDeliveryBuffer(&pOut, &start, &stop, flags);
    if (FAILED(hr)) {
        return hr;
    }

    BYTE* pData = NULL;
    if (FAILED(pOut->GetPointer(&pData)) || !pData) {
        pOut->Release();
        return E_FAIL;
    }
    if (pkt.data.size() > static_cast<size_t>(pOut->GetSize()) ||
        pkt.data.size() > LONG_MAX) {
        pOut->Release();
        return VFW_E_BUFFER_OVERFLOW;
    }
    memcpy(pData, pkt.data.data(), pkt.data.size());
    pOut->SetActualDataLength((long)pkt.data.size());
    pOut->SetTime(&start, &stop);
    if (m_firstPkt) {
        pOut->SetSyncPoint(TRUE);
        pOut->SetDiscontinuity(TRUE);
        m_firstPkt = false;
    }

    hr = m_pOutput->Deliver(pOut);
    pOut->Release();
    return hr;
}

// Not used: we override Receive() and feed frames to ffmpeg ourselves.
HRESULT CFFmpegEncoder::Transform(IMediaSample* /*pIn*/, IMediaSample* /*pOut*/)
{
    return S_OK;
}

// ---------------------------------------------------------------------------
// End of stream: flush encoder and drain remaining packets
// ---------------------------------------------------------------------------

HRESULT CFFmpegEncoder::EndOfStream()
{
    if (m_hChildStdinW) {
        CloseHandle(m_hChildStdinW);
        m_hChildStdinW = NULL;
    }
    if (m_hThread && WaitForSingleObject(m_hThread, 20000) != WAIT_OBJECT_0) {
        if (m_hProc) {
            TerminateProcess(m_hProc, 1);
        }
        WaitForSingleObject(m_hThread, 2000);
    }
    const HRESULT drain = DrainQueue();
    DWORD exitCode = STILL_ACTIVE;
    if (m_hProc && WaitForSingleObject(m_hProc, 5000) == WAIT_OBJECT_0) {
        GetExitCodeProcess(m_hProc, &exitCode);
    }
    m_completed = SUCCEEDED(drain) && exitCode == 0;
    const HRESULT eos = CTransformFilter::EndOfStream();
    return FAILED(drain) ? drain : eos;
}

// ---------------------------------------------------------------------------
// Annex B reader thread
// ---------------------------------------------------------------------------

DWORD WINAPI CFFmpegEncoder::ReaderThreadProc(LPVOID param)
{
    CFFmpegEncoder* pThis = (CFFmpegEncoder*)param;
    pThis->ReaderLoop();
    return 0;
}

void CFFmpegEncoder::ReaderLoop()
{
    BYTE buf[65536];
    for (;;) {
        DWORD rd = 0;
        if (!ReadFile(m_hChildStdoutR, buf, sizeof(buf), &rd, NULL) || rd == 0) {
            break;
        }
        OnRead(buf, rd);
    }
    {
        CAutoLock lock(&m_lock);
        FlushGroup(true);
    }
    m_readerDone.store(true);
}

void CFFmpegEncoder::OnRead(const BYTE* data, size_t len)
{
    CAutoLock lock(&m_lock);
    m_inBuf.insert(m_inBuf.end(), data, data + len);
    ParseAnnexB();
}

bool CFFmpegEncoder::NalIsVcl(const BYTE* nal, size_t len) const
{
    if (len == 0) {
        return false;
    }
    BYTE h = nal[0];
    if ((h & 0x80) != 0) {
        return false;
    }
    if (m_outMux == L"h264") {
        const BYTE type = h & 0x1F;
        return type >= 1 && type <= 5;
    }
    if (m_outMux == L"hevc") {
        return ((h >> 1) & 0x3F) <= 31;
    }
    return true;
}

void CFFmpegEncoder::ParseAnnexB()
{
    auto findStartCode = [&](size_t from, size_t* codeLen) -> size_t {
        for (size_t i = from; i + 2 < m_inBuf.size(); i++) {
            if (m_inBuf[i] == 0 && m_inBuf[i + 1] == 0) {
                if (m_inBuf[i + 2] == 1) {
                    if (i > 0 && m_inBuf[i - 1] == 0) {
                        *codeLen = 4;
                        return i - 1;
                    }
                    *codeLen = 3;
                    return i;
                }
                if (i + 3 < m_inBuf.size() && m_inBuf[i + 2] == 0 &&
                    m_inBuf[i + 3] == 1) {
                    *codeLen = 4;
                    return i;
                }
            }
        }
        return std::string::npos;
    };

    size_t codeLen = 0;
    size_t first = findStartCode(0, &codeLen);
    if (first == std::string::npos) {
        // Bound garbage if a failed encoder writes text or another format to
        // stdout. Valid Annex B streams find a start code in their first KB.
        if (m_inBuf.size() > 1024 * 1024) {
            m_inBuf.clear();
        }
        return;
    }
    size_t nalStart = first + codeLen;
    size_t keepFrom = first;

    for (;;) {
        size_t nextCodeLen = 0;
        size_t next = findStartCode(nalStart, &nextCodeLen);
        if (next == std::string::npos) {
            break;
        }
        if (next > nalStart) {
            const BYTE* nal = m_inBuf.data() + nalStart;
            size_t nalLen = next - nalStart;
            // Strip trailing zero bytes that belong to the next start code.
            while (nalLen > 0 && nal[nalLen - 1] == 0) {
                nalLen--;
            }
            if (nalLen > 0) {
                PushNal(nal, nalLen);
            }
        }
        keepFrom = next;
        codeLen = nextCodeLen;
        nalStart = next + codeLen;
    }

    // Keep the final start code and partial NAL for the next pipe read.
    m_inBuf.erase(m_inBuf.begin(), m_inBuf.begin() + keepFrom);
}

void CFFmpegEncoder::PushNal(const BYTE* nal, size_t len)
{
    if (!nal || len == 0) {
        return;
    }
    const bool vcl = NalIsVcl(nal, len);
    if (!vcl && m_groupHasVcl) {
        FlushGroup(false);
    }
    static const BYTE startCode[] = {0, 0, 0, 1};
    m_group.insert(m_group.end(), startCode, startCode + ARRAYSIZE(startCode));
    m_group.insert(m_group.end(), nal, nal + len);
    if (vcl) {
        m_groupHasVcl = true;
        // ffmpeg's bundled encoders emit one VCL NAL per frame by default.
        FlushGroup(false);
    }
}

void CFFmpegEncoder::FlushGroup(bool atEof)
{
    if (atEof && !m_inBuf.empty()) {
        size_t start = std::string::npos;
        size_t codeLen = 0;
        for (size_t i = 0; i + 2 < m_inBuf.size(); ++i) {
            if (m_inBuf[i] == 0 && m_inBuf[i + 1] == 0 &&
                m_inBuf[i + 2] == 1) {
                start = i;
                codeLen = 3;
                if (i > 0 && m_inBuf[i - 1] == 0) {
                    start = i - 1;
                    codeLen = 4;
                }
                break;
            }
            if (i + 3 < m_inBuf.size() && m_inBuf[i] == 0 &&
                m_inBuf[i + 1] == 0 && m_inBuf[i + 2] == 0 &&
                m_inBuf[i + 3] == 1) {
                start = i;
                codeLen = 4;
                break;
            }
        }
        if (start != std::string::npos && start + codeLen < m_inBuf.size()) {
            size_t end = m_inBuf.size();
            while (end > start + codeLen && m_inBuf[end - 1] == 0) {
                --end;
            }
            if (end > start + codeLen) {
                std::vector<BYTE> tail(m_inBuf.begin() + start + codeLen,
                                       m_inBuf.begin() + end);
                m_inBuf.clear();
                PushNal(tail.data(), tail.size());
            }
        }
        m_inBuf.clear();
    }
    if (!m_group.empty()) {
        Packet pkt;
        pkt.data.swap(m_group);
        m_group.clear();
        m_groupHasVcl = false;
        m_pktQueue.push_back(std::move(pkt));
    }
}
