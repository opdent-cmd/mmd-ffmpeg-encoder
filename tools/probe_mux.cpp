#include <windows.h>
#include <dshow.h>
#include <stdio.h>

// Minimal stub output pin with QueryAccept=S_OK (like a well-behaved encoder).
class CStubPin : public IPin
{
    LONG m_ref = 1;
public:
    STDMETHODIMP QueryInterface(REFIID iid, void** ppv) override
    {
        if (iid == IID_IUnknown || iid == IID_IPin) { *ppv = this; AddRef(); return S_OK; }
        *ppv = NULL; return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_ref); }
    STDMETHODIMP_(ULONG) Release() override
    {
        LONG r = InterlockedDecrement(&m_ref);
        if (r == 0) delete this;
        return r;
    }
    STDMETHODIMP Connect(IPin*, const AM_MEDIA_TYPE*) override { return E_NOTIMPL; }
    STDMETHODIMP ReceiveConnection(IPin*, const AM_MEDIA_TYPE*) override { return E_NOTIMPL; }
    STDMETHODIMP Disconnect() override { return S_OK; }
    STDMETHODIMP ConnectedTo(IPin** pp) override { *pp = NULL; return VFW_E_NOT_CONNECTED; }
    STDMETHODIMP ConnectionMediaType(AM_MEDIA_TYPE* pmt) override
    { ZeroMemory(pmt, sizeof(*pmt)); return VFW_E_NOT_CONNECTED; }
    STDMETHODIMP QueryPinInfo(PIN_INFO* pi) override
    {
        ZeroMemory(pi, sizeof(*pi));
        pi->dir = PINDIR_OUTPUT;
        wcscpy_s(pi->achName, L"Stub");
        return S_OK;
    }
    STDMETHODIMP QueryDirection(PIN_DIRECTION* d) override { *d = PINDIR_OUTPUT; return S_OK; }
    STDMETHODIMP QueryId(LPWSTR* id) override
    {
        *id = (LPWSTR)CoTaskMemAlloc(6 * sizeof(WCHAR));
        wcscpy_s(*id, 6, L"Stub");
        return S_OK;
    }
    STDMETHODIMP QueryAccept(const AM_MEDIA_TYPE*) override { return S_OK; }
    STDMETHODIMP EnumMediaTypes(IEnumMediaTypes** pe) override { *pe = NULL; return E_NOTIMPL; }
    STDMETHODIMP QueryInternalConnections(IPin** ap, ULONG* n) override { *n = 0; return E_NOTIMPL; }
    STDMETHODIMP EndOfStream() override { return S_OK; }
    STDMETHODIMP BeginFlush() override { return S_OK; }
    STDMETHODIMP EndFlush() override { return S_OK; }
    STDMETHODIMP NewSegment(REFERENCE_TIME, REFERENCE_TIME, double) override { return S_OK; }
};

static IPin* GetPin(IBaseFilter* pF, PIN_DIRECTION dir, int index)
{
    IEnumPins* pEnum = NULL;
    pF->EnumPins(&pEnum);
    if (!pEnum) return NULL;
    IPin* pPin = NULL;
    int seen = 0;
    while (pEnum->Next(1, &pPin, NULL) == S_OK) {
        PIN_INFO pi;
        if (SUCCEEDED(pPin->QueryPinInfo(&pi))) {
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

static void TestType(IPin* pIn, DWORD fcc, const GUID& subtype, const WCHAR* name,
                     ULONG sampleSize, DWORD cbExtra, LONG biHeight, BOOL fixed)
{
    BYTE fmt[256];
    ZeroMemory(fmt, sizeof(fmt));
    VIDEOINFOHEADER* vih = (VIDEOINFOHEADER*)fmt;
    vih->rcSource = RECT{0, 0, 320, 240};
    vih->rcTarget = RECT{0, 0, 320, 240};
    vih->AvgTimePerFrame = 333333;
    vih->bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    vih->bmiHeader.biWidth = 320;
    vih->bmiHeader.biHeight = biHeight;
    vih->bmiHeader.biPlanes = 1;
    vih->bmiHeader.biBitCount = 24;
    vih->bmiHeader.biCompression = fcc;
    vih->bmiHeader.biSizeImage = 230400;

    AM_MEDIA_TYPE mt;
    ZeroMemory(&mt, sizeof(mt));
    mt.majortype = MEDIATYPE_Video;
    mt.subtype = subtype;
    mt.bFixedSizeSamples = fixed;
    mt.bTemporalCompression = TRUE;
    mt.lSampleSize = sampleSize;
    mt.formattype = FORMAT_VideoInfo;
    mt.pbFormat = fmt;
    mt.cbFormat = sizeof(VIDEOINFOHEADER) + cbExtra;

    HRESULT hq = pIn->QueryAccept(&mt);
    CStubPin* stub = new CStubPin();
    HRESULT hr = pIn->ReceiveConnection(stub, &mt);
    wprintf(L"%-22s sz=%-6u extra=%-3u h=%ld fixed=%d  QueryAccept=0x%08X ReceiveConnection=0x%08X\n",
            name, sampleSize, cbExtra, biHeight, fixed ? 1 : 0, hq, hr);
    stub->Release();
}

static GUID MakeSub(DWORD fcc)
{
    return GUID{fcc, 0x0000, 0x0010, {0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71}};
}

int wmain(int argc, wchar_t** argv)
{
    setvbuf(stdout, NULL, _IONBF, 0);
    bool withWriter = !(argc > 1 && wcscmp(argv[1], L"--nowriter") == 0);
    CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);

    IGraphBuilder* g = NULL;
    IBaseFilter* mux = NULL;
    IBaseFilter* wr = NULL;
    CoCreateInstance(CLSID_FilterGraph, NULL, CLSCTX_INPROC_SERVER, IID_IGraphBuilder, (void**)&g);
    CoCreateInstance(CLSID_AviDest, NULL, CLSCTX_INPROC_SERVER, IID_IBaseFilter, (void**)&mux);
    g->AddFilter(mux, L"AVI Mux");
    if (withWriter) {
        CoCreateInstance(CLSID_FileWriter, NULL, CLSCTX_INPROC_SERVER, IID_IBaseFilter, (void**)&wr);
        g->AddFilter(wr, L"File Writer");
        IFileSinkFilter* sink = NULL;
        wr->QueryInterface(IID_IFileSinkFilter, (void**)&sink);
        sink->SetFileName(L"probe_mux_out.avi", NULL);
        sink->Release();
        IPin* muxOut = GetPin(mux, PINDIR_OUTPUT, 0);
        IPin* wrIn = GetPin(wr, PINDIR_INPUT, 0);
        HRESULT h = g->Connect(muxOut, wrIn);
        wprintf(L"mux->writer connect: 0x%08X (withWriter=%d)\n", h, withWriter ? 1 : 0);
        if (muxOut) muxOut->Release();
        if (wrIn) wrIn->Release();
    }

    IPin* pIn = GetPin(mux, PINDIR_INPUT, 0);
    if (!pIn) { wprintf(L"no input pin\n"); return 1; }

    GUID h264 = MakeSub(mmioFOURCC('H','2','6','4'));
    GUID hevc = MakeSub(mmioFOURCC('H','E','V','C'));
    GUID mjpg = MakeSub(mmioFOURCC('M','J','P','G'));

    wprintf(L"--- H264 variants ---\n");
    TestType(pIn, mmioFOURCC('H','2','6','4'), h264, L"H264", 0, 0, -240, FALSE);
    TestType(pIn, mmioFOURCC('H','2','6','4'), h264, L"H264", 1, 0, -240, FALSE);
    TestType(pIn, mmioFOURCC('H','2','6','4'), h264, L"H264", 230400, 0, -240, FALSE);
    TestType(pIn, mmioFOURCC('H','2','6','4'), h264, L"H264", 0, 48, -240, FALSE);
    TestType(pIn, mmioFOURCC('H','2','6','4'), h264, L"H264", 1, 48, -240, FALSE);
    TestType(pIn, mmioFOURCC('H','2','6','4'), h264, L"H264", 0, 0, 240, FALSE);
    TestType(pIn, mmioFOURCC('H','2','6','4'), h264, L"H264", 0, 0, -240, TRUE);
    TestType(pIn, mmioFOURCC('a','v','c','1'), h264, L"avc1", 0, 0, -240, FALSE);
    wprintf(L"--- HEVC variants ---\n");
    TestType(pIn, mmioFOURCC('H','E','V','C'), hevc, L"HEVC", 0, 0, -240, FALSE);
    TestType(pIn, mmioFOURCC('H','E','V','C'), hevc, L"HEVC", 1, 0, -240, FALSE);
    TestType(pIn, mmioFOURCC('h','v','c','1'), hevc, L"hvc1", 0, 0, -240, FALSE);
    wprintf(L"--- MJPEG reference ---\n");
    TestType(pIn, mmioFOURCC('M','J','P','G'), mjpg, L"MJPEG", 0, 0, -240, FALSE);
    TestType(pIn, mmioFOURCC('M','J','P','G'), mjpg, L"MJPEG", 1, 48, -240, FALSE);

    pIn->Release();
    if (mux) mux->Release();
    if (wr) wr->Release();
    if (g) g->Release();
    CoUninitialize();
    return 0;
}
