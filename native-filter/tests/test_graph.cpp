#include <windows.h>
#include <dshow.h>
#include <stdio.h>

// Definition of our CLSID for the test harness (header only declares it).
EXTERN_C const GUID CLSID_FFmpegEncoder = {
    0xD79D43B2, 0xF005, 0x40A4,
    { 0xBE, 0x18, 0xAF, 0xD1, 0x9C, 0x03, 0xE6, 0xE6 }
};

// MJPEG Compressor (reference encoder) from the video compressor category.
static const GUID CLSID_MJPGCompressor = {0xB80AB0A0, 0x7416, 0x11D2, {0x9E,0xEB,0x00,0x60,0x08,0x03,0x9E,0x37}};
static const GUID CLSID_SampleGrabberLocal = {0xC1F400A0, 0x3F08, 0x11D3, {0x9F,0x0B,0x00,0x60,0x08,0x03,0x9E,0x37}};
static const GUID CLSID_MMDxShow = {0x2F1713B8, 0xDD1F, 0x4186, {0x93,0xBE,0xFD,0xA5,0x0B,0xF6,0x87,0xC7}};
static const GUID IID_MMDxShow = {0xECFAB031, 0x72BA, 0x4120, {0xB9,0xF7,0x8A,0x3D,0x5F,0xD3,0x8D,0xEC}};
static const GUID IID_ISampleGrabberLocal = {0x6B652FFF, 0x11FE, 0x4FCE, {0x92,0xAD,0x02,0x66,0xB5,0xD7,0xC7,0x8F}};
static const GUID MEDIASUBTYPE_MMDXSHOW_RGB32 = {0x773C9AC0, 0x3274, 0x11D0, {0xB7,0x24,0x00,0xAA,0x00,0x6C,0x1A,0x01}};

typedef HRESULT (STDMETHODCALLTYPE *PFN_MMDXSHOW_SET)(void*, void*, int, float);

static FILE* g_dbg = NULL;
static bool g_smart = false;

static LONG WINAPI CrashHandler(EXCEPTION_POINTERS* ep)
{
    if (g_dbg) {
        fwprintf(g_dbg, L"CRASH code=0x%08X addr=0x%p\n",
                 ep->ExceptionRecord->ExceptionCode,
                 ep->ExceptionRecord->ExceptionAddress);
        fflush(g_dbg);
    }
    return EXCEPTION_EXECUTE_HANDLER;
}

static void Fail(const WCHAR* what, HRESULT hr)
{
    wprintf(L"FAIL %s: 0x%08X\n", what, hr);
    if (g_dbg) {
        fwprintf(g_dbg, L"FAIL %s: 0x%08X\n", what, hr);
        fflush(g_dbg);
    }
}

static IPin* GetPin(IBaseFilter* pFilter, PIN_DIRECTION dir, int index)
{
    IEnumPins* pEnum = NULL;
    if (FAILED(pFilter->EnumPins(&pEnum))) {
        return NULL;
    }
    IPin* pPin = NULL;
    int seen = 0;
    while (pEnum->Next(1, &pPin, NULL) == S_OK) {
        PIN_INFO pi;
        HRESULT hr = pPin->QueryPinInfo(&pi);
        if (SUCCEEDED(hr)) {
            if (pi.dir == dir && seen++ == index) {
                pi.pFilter->Release();
                pEnum->Release();
                return pPin;
            }
            pi.pFilter->Release();
        }
        pPin->Release();
    }
    pEnum->Release();
    return NULL;
}

static HRESULT AddFilter(IGraphBuilder* pGraph, REFCLSID clsid, const WCHAR* name,
                         IBaseFilter** ppFilter)
{
    if (!pGraph || !ppFilter) {
        return E_POINTER;
    }
    *ppFilter = NULL;
    HRESULT hr = CoCreateInstance(clsid, NULL, CLSCTX_INPROC_SERVER,
                                  IID_IBaseFilter, (void**)ppFilter);
    if (FAILED(hr)) {
        return hr;
    }
    hr = pGraph->AddFilter(*ppFilter, name);
    if (FAILED(hr)) {
        (*ppFilter)->Release();
        *ppFilter = NULL;
    }
    return hr;
}

static HRESULT ConnectPins(IGraphBuilder* pGraph, IBaseFilter* pA, IBaseFilter* pB)
{
    if (!pGraph || !pA || !pB) {
        return E_POINTER;
    }
    for (int i = 0;; i++) {
        IPin* pOut = GetPin(pA, PINDIR_OUTPUT, i);
        if (!pOut) {
            break;
        }
        for (int j = 0;; j++) {
            IPin* pIn = GetPin(pB, PINDIR_INPUT, j);
            if (!pIn) {
                break;
            }
            PIN_INFO piA, piB;
            WCHAR na[64], nb[64];
            wcscpy_s(na, L"?");
            wcscpy_s(nb, L"?");
            if (SUCCEEDED(pOut->QueryPinInfo(&piA))) {
                wcsncpy_s(na, 64, piA.achName, _TRUNCATE);
                if (piA.pFilter) piA.pFilter->Release();
            }
            if (SUCCEEDED(pIn->QueryPinInfo(&piB))) {
                wcsncpy_s(nb, 64, piB.achName, _TRUNCATE);
                if (piB.pFilter) piB.pFilter->Release();
            }
            wprintf(L"[dbg] Connect %s -> %s\n", na, nb);
            HRESULT hr = g_smart ? pGraph->Connect(pOut, pIn)
                                 : pGraph->ConnectDirect(pOut, pIn, NULL);
            pIn->Release();
            if (SUCCEEDED(hr)) {
                pOut->Release();
                return hr;
            }
        }
        pOut->Release();
    }
    return VFW_E_CANNOT_CONNECT;
}

static void PrintMediaType(IPin* pPin, const WCHAR* label)
{
    AM_MEDIA_TYPE mt;
    ZeroMemory(&mt, sizeof(mt));
    if (SUCCEEDED(pPin->ConnectionMediaType(&mt))) {
        WCHAR name[64];
        StringFromGUID2(mt.subtype, name, 64);
        wprintf(L"%s subtype: %s\n", label, name);
        if (mt.pUnk) {
            mt.pUnk->Release();
        }
        if (mt.pbFormat) {
            CoTaskMemFree(mt.pbFormat);
        }
    }
}

static void ListCompressors()
{
    HRESULT hr = CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);
    ICreateDevEnum* pDevEnum = NULL;
    hr = CoCreateInstance(CLSID_SystemDeviceEnum, NULL, CLSCTX_INPROC_SERVER,
                          IID_ICreateDevEnum, (void**)&pDevEnum);
    if (FAILED(hr)) {
        Fail(L"CoCreateInstance(CLSID_SystemDeviceEnum)", hr);
        CoUninitialize();
        return;
    }
    IEnumMoniker* pEnum = NULL;
    hr = pDevEnum->CreateClassEnumerator(CLSID_VideoCompressorCategory, &pEnum, 0);
    if (hr == S_OK && pEnum) {
        wprintf(L"--- video compressors ---\n");
        IMoniker* pMoniker = NULL;
        while (pEnum->Next(1, &pMoniker, NULL) == S_OK) {
            IPropertyBag* pBag = NULL;
            if (SUCCEEDED(pMoniker->BindToStorage(0, 0, IID_IPropertyBag,
                                                  (void**)&pBag))) {
                VARIANT var;
                VariantInit(&var);
                if (SUCCEEDED(pBag->Read(L"FriendlyName", &var, NULL))) {
                    wprintf(L"  %s\n", var.bstrVal);
                    VariantClear(&var);
                }
                pBag->Release();
            }
            pMoniker->Release();
        }
        pEnum->Release();
    } else {
        wprintf(L"(no video compressors enumerated, hr=0x%08X)\n", hr);
    }
    pDevEnum->Release();
    CoUninitialize();
}

int wmain(int argc, wchar_t** argv)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    SetUnhandledExceptionFilter(CrashHandler);
    FILE* logFile = NULL;
    if (_wfopen_s(&logFile, L"test_graph.log", L"w") == 0) {
        g_dbg = logFile;
    }
    auto LOG = [&](const WCHAR* s) {
        wprintf(L"%s\n", s);
        if (g_dbg) {
            fwprintf(g_dbg, L"%s\n", s);
            fflush(g_dbg);
        }
    };
    if (argc > 1 && wcscmp(argv[1], L"--list") == 0) {
        if (g_dbg) fclose(g_dbg);
        ListCompressors();
        return 0;
    }
    bool useMjpg = false, rawDirect = false, smart = false, diag = false;
    bool nosink = false, grabber = false, grabberDirect = false, xshow = false;
    bool sourceDirect = false;
    for (int ai = 3; ai < argc; ai++) {
        if (wcscmp(argv[ai], L"--mjpg") == 0) useMjpg = true;
        if (wcscmp(argv[ai], L"--raw") == 0) rawDirect = true;
        if (wcscmp(argv[ai], L"--smart") == 0) smart = true;
        if (wcscmp(argv[ai], L"--diag") == 0) diag = true;
        if (wcscmp(argv[ai], L"--nosink") == 0) nosink = true;
        if (wcscmp(argv[ai], L"--grabber") == 0) grabber = true;
        if (wcscmp(argv[ai], L"--grabber-direct") == 0) grabberDirect = true;
        if (wcscmp(argv[ai], L"--xshow") == 0) xshow = true;
        if (wcscmp(argv[ai], L"--source-direct") == 0) sourceDirect = true;
    }
    if (grabberDirect) {
        grabber = true;
    }
    if (smart) {
        wprintf(L"using IGraphBuilder::Connect (smart connect)\n");
    }
    g_smart = smart;
    if (argc < 3) {
        wprintf(L"usage: test_graph.exe <input.avi> <output.avi>\n");
        wprintf(L"       test_graph.exe --list\n");
        if (g_dbg) fclose(g_dbg);
        return 1;
    }

    LOG(L"CoInitializeEx");
    HRESULT hr = CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);
    if (FAILED(hr)) {
        Fail(L"CoInitializeEx", hr);
        return 1;
    }

    IGraphBuilder* pGraph = NULL;
    LOG(L"CoCreateInstance(FilterGraph)");
    hr = CoCreateInstance(CLSID_FilterGraph, NULL, CLSCTX_INPROC_SERVER,
                          IID_IGraphBuilder, (void**)&pGraph);
    if (FAILED(hr)) {
        Fail(L"CoCreateInstance(FilterGraph)", hr);
        CoUninitialize();
        return 1;
    }

    IBaseFilter* pSrc = NULL;
    IBaseFilter* pSplit = NULL;
    IBaseFilter* pDec = NULL;
    IBaseFilter* pEnc = NULL;
    IBaseFilter* pGrab = NULL;
    IBaseFilter* pMux = NULL;
    IBaseFilter* pWr = NULL;
    IFileSourceFilter* pLoad = NULL;
    IFileSinkFilter* pSink = NULL;
    IMediaControl* pMC = NULL;

    if (xshow) {
        hr = AddFilter(pGraph, CLSID_MMDxShow, L"MMDxShow", &pSrc);
        LOG(L"added MMDxShow");
        if (FAILED(hr)) { Fail(L"AddFilter(MMDxShow)", hr); goto done; }
        // Configure it like MMD does: Set(BITMAPINFOHEADER*, 40, fps).
        void* x = NULL;
        if (SUCCEEDED(pSrc->QueryInterface(IID_MMDxShow, &x)) && x) {
            BITMAPINFOHEADER bih;
            ZeroMemory(&bih, sizeof(bih));
            bih.biSize = sizeof(bih);
            bih.biWidth = 640;
            bih.biHeight = -480;
            bih.biPlanes = 1;
            bih.biBitCount = 32;
            bih.biSizeImage = 640 * 480 * 4;
            void** vt = *(void***)x;
            HRESULT hs = ((PFN_MMDXSHOW_SET)vt[0x18/8])(x, &bih, 40, 30.0f);
            wprintf(L"MMDxShow Set: 0x%08X\n", hs);
            ((IUnknown*)x)->Release();
        }
    } else {
        hr = AddFilter(pGraph, CLSID_AsyncReader, L"File Source", &pSrc);
        LOG(L"added File Source");
        if (FAILED(hr)) { Fail(L"AddFilter(File Source)", hr); goto done; }
        hr = AddFilter(pGraph, CLSID_AviSplitter, L"AVI Splitter", &pSplit);
        LOG(L"added AVI Splitter");
        if (FAILED(hr)) { Fail(L"AddFilter(AVI Splitter)", hr); goto done; }
    }
    // The raw RGB test AVI needs no decompressor; connect the splitter
    // straight into the encoder (same topology MMD uses with its DIB source).
    LOG(useMjpg ? L"using MJPEG Compressor reference"
        : rawDirect ? L"raw: splitter->mux direct"
                    : L"skipped AVI Decompressor");
    if (grabber && !rawDirect) {
        hr = AddFilter(pGraph, CLSID_SampleGrabberLocal, L"Sample Grabber", &pGrab);
        LOG(L"added Sample Grabber");
        if (FAILED(hr)) { Fail(L"AddFilter(SampleGrabber)", hr); goto done; }
        // Accept MMDxShow's 32bpp DIB type so the source can connect directly.
        void* sg = NULL;
        if (SUCCEEDED(pGrab->QueryInterface(IID_ISampleGrabberLocal, &sg)) && sg) {
            // Exactly what MMD does: {majortype=Video, subtype=MMDxShow RGB32},
            // no format block.
            AM_MEDIA_TYPE mt;
            ZeroMemory(&mt, sizeof(mt));
            mt.majortype = MEDIATYPE_Video;
            mt.subtype = MEDIASUBTYPE_MMDXSHOW_RGB32;
            mt.bFixedSizeSamples = TRUE;
            mt.lSampleSize = 1;
            void** vt = *(void***)sg;
            HRESULT hmt = ((HRESULT (STDMETHODCALLTYPE*)(void*, const AM_MEDIA_TYPE*))vt[0x20/8])(sg, &mt);
            wprintf(L"SampleGrabber SetMediaType(MMD style): 0x%08X\n", hmt);
            // Also try with a full format block (640x480 32bpp).
            BYTE fmt[88];
            ZeroMemory(fmt, sizeof(fmt));
            VIDEOINFOHEADER* vih = (VIDEOINFOHEADER*)fmt;
            vih->rcSource = RECT{0, 0, 640, 480};
            vih->rcTarget = RECT{0, 0, 640, 480};
            vih->AvgTimePerFrame = 333333;
            vih->bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
            vih->bmiHeader.biWidth = 640;
            vih->bmiHeader.biHeight = -480;
            vih->bmiHeader.biPlanes = 1;
            vih->bmiHeader.biBitCount = 32;
            vih->bmiHeader.biSizeImage = 1228800;
            AM_MEDIA_TYPE mt2;
            ZeroMemory(&mt2, sizeof(mt2));
            mt2.majortype = MEDIATYPE_Video;
            mt2.subtype = MEDIASUBTYPE_MMDXSHOW_RGB32;
            mt2.bFixedSizeSamples = TRUE;
            mt2.lSampleSize = 1228800;
            mt2.formattype = FORMAT_VideoInfo;
            mt2.pbFormat = fmt;
            mt2.cbFormat = sizeof(fmt);
            HRESULT hmt3 = ((HRESULT (STDMETHODCALLTYPE*)(void*, const AM_MEDIA_TYPE*))vt[0x20/8])(sg, &mt2);
            wprintf(L"SampleGrabber SetMediaType(full): 0x%08X\n", hmt3);
            ((IUnknown*)sg)->Release();
        }
    }
    if (!rawDirect) {
        LOG(useMjpg ? L"CoCreateInstance(MJPEG Compressor)" : L"CoCreateInstance(FFmpegEncoder)");
        hr = CoCreateInstance(useMjpg ? CLSID_MJPGCompressor : CLSID_FFmpegEncoder,
                              NULL, CLSCTX_INPROC_SERVER,
                              IID_IBaseFilter, (void**)&pEnc);
        if (FAILED(hr)) {
            Fail(useMjpg ? L"CoCreateInstance(MJPEG)" : L"CoCreateInstance(FFmpegEncoder)", hr);
            goto done;
        }
        LOG(L"graph AddFilter(encoder)");
        hr = pGraph->AddFilter(pEnc, useMjpg ? L"MJPEG Compressor" : L"FFmpeg Encoder");
        if (FAILED(hr)) {
            Fail(L"AddFilter(FFmpegEncoder)", hr);
            goto done;
        }
    }
    LOG(useMjpg ? L"added MJPEG" : rawDirect ? L"no encoder (raw)" : L"added FFmpegEncoder");
    if (!nosink) {
        hr = AddFilter(pGraph, CLSID_AviDest, L"AVI Mux", &pMux);
        LOG(L"added AVI Mux");
        if (FAILED(hr)) { Fail(L"AddFilter(AVI Mux)", hr); goto done; }
        hr = AddFilter(pGraph, CLSID_FileWriter, L"File Writer", &pWr);
        LOG(L"added File Writer");
        if (FAILED(hr)) { Fail(L"AddFilter(File Writer)", hr); goto done; }
    }

    if (!xshow) {
        pSrc->QueryInterface(IID_IFileSourceFilter, (void**)&pLoad);
        hr = pLoad ? pLoad->Load(argv[1], NULL) : E_NOINTERFACE;
        LOG(L"loaded input");
        if (FAILED(hr)) { Fail(L"Load input AVI", hr); goto done; }
    }

    if (!nosink) {
        pWr->QueryInterface(IID_IFileSinkFilter, (void**)&pSink);
        hr = pSink ? pSink->SetFileName(argv[2], NULL) : E_NOINTERFACE;
        LOG(L"set output filename");
        if (FAILED(hr)) { Fail(L"SetFileName output", hr); goto done; }
    }

    if (!xshow) {
        hr = ConnectPins(pGraph, pSrc, pSplit);
        if (FAILED(hr)) { Fail(L"connect source->splitter", hr); goto done; }
        LOG(L"[ok] source->splitter");
    }
    if (!nosink) {
        hr = ConnectPins(pGraph, pMux, pWr);
        if (FAILED(hr)) { Fail(L"connect AVI Mux->File Writer", hr); goto done; }
        LOG(L"[ok] AVI Mux->File Writer");
    }
    if (!rawDirect) {
        if (grabber) {
            if (sourceDirect) {
                IPin* pSOut = GetPin(xshow ? pSrc : pSplit, PINDIR_OUTPUT, 0);
                IPin* pGIn = GetPin(pGrab, PINDIR_INPUT, 0);
                hr = pSOut && pGIn ? pSOut->Connect(pGIn, NULL) : E_FAIL;
                wprintf(L"sourceOut->grabberIn direct = 0x%08X\n", hr);
                if (pSOut) pSOut->Release();
                if (pGIn) pGIn->Release();
            } else {
                hr = ConnectPins(pGraph, xshow ? pSrc : pSplit, pGrab);
            }
            if (FAILED(hr)) { Fail(L"connect source->SampleGrabber", hr); goto done; }
            LOG(L"[ok] source->SampleGrabber");
            if (grabberDirect) {
                IPin* pGOut = GetPin(pGrab, PINDIR_OUTPUT, 0);
                IPin* pEIn = GetPin(pEnc, PINDIR_INPUT, 0);
                hr = pGOut && pEIn ? pGOut->Connect(pEIn, NULL) : E_FAIL;
                wprintf(L"grabberOut->Connect(encIn) direct = 0x%08X\n", hr);
                if (pGOut) pGOut->Release();
                if (pEIn) pEIn->Release();
            } else {
                hr = ConnectPins(pGraph, pGrab, pEnc);
            }
        } else {
            hr = ConnectPins(pGraph, xshow ? pSrc : pSplit, pEnc);
        }
        if (FAILED(hr)) { Fail(L"connect splitter->FFmpegEncoder", hr); goto done; }
        LOG(L"[ok] splitter->FFmpegEncoder");
    }
    if (!rawDirect && !nosink) {
        hr = ConnectPins(pGraph, pEnc, pMux);
        if (FAILED(hr)) { Fail(L"connect FFmpegEncoder->AVI Mux", hr); goto done; }
        LOG(L"[ok] FFmpegEncoder->AVI Mux");
    } else if (!nosink) {
        hr = ConnectPins(pGraph, xshow ? pSrc : pSplit, pMux);
        if (FAILED(hr)) { Fail(L"connect source->AVI Mux (raw)", hr); goto done; }
        LOG(L"[ok] source->AVI Mux (raw)");
    }

    if (pEnc) {
        IPin* pEncIn = GetPin(pEnc, PINDIR_INPUT, 0);
        if (pEncIn) {
            PrintMediaType(pEncIn, L"encoder input");
            pEncIn->Release();
        }
    }

    if (diag && pEnc) {
        // Print connection state of the encoder pins.
        IPin* pEncIn2 = GetPin(pEnc, PINDIR_INPUT, 0);
        if (pEncIn2) {
            IPin* pPeer = NULL;
            if (SUCCEEDED(pEncIn2->ConnectedTo(&pPeer))) {
                PIN_INFO pi;
                if (SUCCEEDED(pPeer->QueryPinInfo(&pi))) {
                    wprintf(L"[diag] encoder input connected to pin \"%s\"\n", pi.achName);
                    if (pi.pFilter) pi.pFilter->Release();
                }
                pPeer->Release();
            } else {
                wprintf(L"[diag] encoder input NOT connected\n");
            }
            pEncIn2->Release();
        }
        IPin* pEncOut2 = GetPin(pEnc, PINDIR_OUTPUT, 0);
        if (pEncOut2) {
            IPin* pPeer = NULL;
            if (SUCCEEDED(pEncOut2->ConnectedTo(&pPeer))) {
                PIN_INFO pi;
                if (SUCCEEDED(pPeer->QueryPinInfo(&pi))) {
                    wprintf(L"[diag] encoder output connected to pin \"%s\"\n", pi.achName);
                    if (pi.pFilter) pi.pFilter->Release();
                }
                pPeer->Release();
            } else {
                wprintf(L"[diag] encoder output NOT connected\n");
            }
            pEncOut2->Release();
        }

        // Real Run flow: Pause all, Run all, then sample filter states.
        IMediaControl* pMC2 = NULL;
        pGraph->QueryInterface(IID_IMediaControl, (void**)&pMC2);
        // First individually Pause each filter to find the failing one.
        IEnumFilters* pEnum0 = NULL;
        if (SUCCEEDED(pGraph->EnumFilters(&pEnum0))) {
            IBaseFilter* pF = NULL;
            int idx = 0;
            while (pEnum0->Next(1, &pF, NULL) == S_OK) {
                FILTER_INFO fi;
                WCHAR name[160];
                ZeroMemory(name, sizeof(name));
                if (SUCCEEDED(pF->QueryFilterInfo(&fi))) {
                    wcsncpy_s(name, 160, fi.achName, _TRUNCATE);
                    if (fi.pGraph) fi.pGraph->Release();
                }
                IMediaFilter* pMF = NULL;
                HRESULT hrP = E_FAIL;
                if (SUCCEEDED(pF->QueryInterface(IID_IMediaFilter, (void**)&pMF))) {
                    hrP = pMF->Pause();
                    pMF->Release();
                }
                wprintf(L"[diag-pause] filter[%d] \"%s\" Pause=0x%08X\n", idx++, name, hrP);
                pF->Release();
            }
            pEnum0->Release();
        }
        HRESULT hrRun = pMC2 ? pMC2->Run() : E_FAIL;
        wprintf(L"[diag] IMediaControl::Run = 0x%08X\n", hrRun);
        Sleep(3000);

        IEnumFilters* pEnum = NULL;
        if (SUCCEEDED(pGraph->EnumFilters(&pEnum))) {
            IBaseFilter* pF = NULL;
            int idx = 0;
            while (pEnum->Next(1, &pF, NULL) == S_OK) {
                FILTER_INFO fi;
                WCHAR name[160];
                ZeroMemory(name, sizeof(name));
                if (SUCCEEDED(pF->QueryFilterInfo(&fi))) {
                    wcsncpy_s(name, 160, fi.achName, _TRUNCATE);
                    if (fi.pGraph) fi.pGraph->Release();
                }
                IMediaFilter* pMF = NULL;
                if (SUCCEEDED(pF->QueryInterface(IID_IMediaFilter, (void**)&pMF))) {
                    FILTER_STATE st = State_Stopped;
                    HRESULT hrS = pMF->GetState(500, &st);
                    wprintf(L"[diag] filter[%d] \"%s\" GetState=0x%08X state=%d\n",
                            idx, name, hrS, (int)st);
                    pMF->Release();
                }
                idx++;
                pF->Release();
            }
            pEnum->Release();
        }
        if (pMC2) {
            pMC2->Stop();
            pMC2->Release();
        }
        if (g_dbg) fclose(g_dbg);
        CoUninitialize();
        return 0;
    }

    pGraph->QueryInterface(IID_IMediaControl, (void**)&pMC);
    hr = pMC ? pMC->Run() : E_NOINTERFACE;
    wprintf(L"Run returned 0x%08X\n", hr);
    LOG(L"run issued");
    if (FAILED(hr)) {
        Fail(L"Run", hr);
        goto done;
    }

    if (xshow) {
        Sleep(4000);
        wprintf(L"xshow run finished after 4s\n");
    } else {
        IMediaEvent* pEv = NULL;
        pGraph->QueryInterface(IID_IMediaEvent, (void**)&pEv);
        if (pEv) {
            long evCode = 0;
            hr = pEv->WaitForCompletion(120000, &evCode);
            wprintf(L"graph event: hr=0x%08X code=%ld\n", hr, evCode);
            pEv->Release();
        }
    }
    if (pMC) {
        pMC->Stop();
    }
    wprintf(L"output written to %s\n", argv[2]);

done:
    if (pMC) pMC->Release();
    if (pLoad) pLoad->Release();
    if (pSink) pSink->Release();
    if (pSrc) pSrc->Release();
    if (pSplit) pSplit->Release();
    if (pGrab) pGrab->Release();
    if (pDec) pDec->Release();
    if (pEnc) pEnc->Release();
    if (pMux) pMux->Release();
    if (pWr) pWr->Release();
    if (pGraph) pGraph->Release();
    if (g_dbg) fclose(g_dbg);
    CoUninitialize();
    return SUCCEEDED(hr) ? 0 : 1;
}
