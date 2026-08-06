#include <windows.h>
#include <dshow.h>
#include <stdio.h>

static const GUID CLSID_MMDxShow = {0x2F1713B8, 0xDD1F, 0x4186, {0x93,0xBE,0xFD,0xA5,0x0B,0xF6,0x87,0xC7}};
static const GUID IID_MMDxShow = {0xECFAB031, 0x72BA, 0x4120, {0xB9,0xF7,0x8A,0x3D,0x5F,0xD3,0x8D,0xEC}};
static const GUID IID_ISampleGrabberLocal = {0x6B652FFF, 0x11FE, 0x4FCE, {0x92,0xAD,0x02,0x66,0xB5,0xD7,0xC7,0x8F}};
static const GUID CLSID_SampleGrabberLocal = {0xC1F400A0, 0x3F08, 0x11D3, {0x9F,0x0B,0x00,0x60,0x08,0x03,0x9E,0x37}};
static const GUID MEDIASUBTYPE_MMDXSHOW_RGB32 = {0x773C9AC0, 0x3274, 0x11D0, {0xB7,0x24,0x00,0xAA,0x00,0x6C,0x1A,0x01}};

// ISampleGrabber from the classic qedit.h (removed from modern SDKs).
struct __declspec(uuid("6B652FFF-11FE-4FCE-92AD-0266B5D7C78F")) ISampleGrabber : public IUnknown
{
    virtual HRESULT STDMETHODCALLTYPE GetConnectedMediaType(AM_MEDIA_TYPE *pType) = 0;
    virtual HRESULT STDMETHODCALLTYPE SetBufferSamples(BOOL bBuffer) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetCurrentBuffer(long *pBufferSize, long *pBuffer) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetCurrentSample(IMediaSample **ppSample) = 0;
    virtual HRESULT STDMETHODCALLTYPE SetOneShot(BOOL bOneShot) = 0;
    virtual HRESULT STDMETHODCALLTYPE SetMediaType(const AM_MEDIA_TYPE *pmt) = 0;
    virtual HRESULT STDMETHODCALLTYPE IsSampleLater(long *plSampleTime) = 0;
};

typedef HRESULT (STDMETHODCALLTYPE *PFN_GETVER)(void*, float*);
typedef HRESULT (STDMETHODCALLTYPE *PFN_SET)(void*, void*, int, float);

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

static void EnumOutTypes(IBaseFilter* f, const WCHAR* label)
{
    IPin* pOut = GetPin(f, PINDIR_OUTPUT, 0);
    if (!pOut) { wprintf(L"%s: no output pin\n", label); return; }
    IEnumMediaTypes* emt = NULL;
    HRESULT hr = pOut->EnumMediaTypes(&emt);
    wprintf(L"%s: EnumMediaTypes hr=0x%08X\n", label, hr);
    if (emt) {
        AM_MEDIA_TYPE* pmt = NULL;
        int n = 0;
        while (emt->Next(1, &pmt, NULL) == S_OK && n < 8) {
            wprintf(L"  type[%d] subtype={%08X-%04X-%04X} fmttype=%08X cb=%u\n",
                    n++, pmt->subtype.Data1, pmt->subtype.Data2, pmt->subtype.Data3,
                    pmt->formattype.Data1, pmt->cbFormat);
            if (pmt->pbFormat && pmt->cbFormat >= 88) {
                const BYTE* p = (const BYTE*)pmt->pbFormat;
                wprintf(L"    fmt[0..87]: ");
                for (int k = 0; k < 88; k++) {
                    wprintf(L"%02X ", p[k]);
                    if ((k % 16) == 15 && k < 87) wprintf(L"\n                 ");
                }
                wprintf(L"\n");
            }
            if (pmt->pbFormat && pmt->cbFormat >= sizeof(BITMAPINFOHEADER)) {
                const VIDEOINFOHEADER* vih = (const VIDEOINFOHEADER*)pmt->pbFormat;
                wprintf(L"    %ldx%ld bpp=%u fps=%d\n",
                        vih->bmiHeader.biWidth, vih->bmiHeader.biHeight,
                        vih->bmiHeader.biBitCount,
                        vih->AvgTimePerFrame ? (int)(10000000 / vih->AvgTimePerFrame) : 0);
            }
            if (pmt->pbFormat) CoTaskMemFree(pmt->pbFormat);
            CoTaskMemFree(pmt);
            pmt = NULL;
        }
        emt->Release();
    }
    pOut->Release();
}

int wmain()
{
    setvbuf(stdout, NULL, _IONBF, 0);
    CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);
    IBaseFilter* f = NULL;
    HRESULT hr = CoCreateInstance(CLSID_MMDxShow, NULL, CLSCTX_INPROC_SERVER, IID_IBaseFilter, (void**)&f);
    wprintf(L"create xShow: 0x%08X\n", hr);
    if (FAILED(hr)) return 1;
    EnumOutTypes(f, L"before config");

    void* x = NULL;
    hr = f->QueryInterface(IID_MMDxShow, &x);
    wprintf(L"QI IMMDxShow: 0x%08X ptr=%p\n", hr, x);
    if (SUCCEEDED(hr) && x) {
        void** vt = *(void***)x;
        float ver = 0;
        HRESULT h2 = ((PFN_GETVER)vt[0x38/8])(x, &ver);
        wprintf(L"GetVersion: hr=0x%08X ver=%f\n", h2, ver);

        // MMD passes a pointer to a BITMAPINFOHEADER, its size (40) and fps.
        BITMAPINFOHEADER bih;
        ZeroMemory(&bih, sizeof(bih));
        bih.biSize = sizeof(bih);
        bih.biWidth = 640;
        bih.biHeight = -480;
        bih.biPlanes = 1;
        bih.biBitCount = 32;
        HRESULT h3 = ((PFN_SET)vt[0x18/8])(x, &bih, 40, 30.0f);
        wprintf(L"Set(&bih,40,30): 0x%08X\n", h3);
        EnumOutTypes(f, L"after Set(&bih,40,30)");

    }
    f->Release();

    // Fresh SampleGrabber for slot mapping.
    IBaseFilter* grab = NULL;
    CoCreateInstance(CLSID_SampleGrabberLocal, NULL, CLSCTX_INPROC_SERVER, IID_IBaseFilter, (void**)&grab);
    void* sg = NULL;
    if (grab && SUCCEEDED(grab->QueryInterface(IID_ISampleGrabberLocal, &sg)) && sg) {
        ISampleGrabber* ig = (ISampleGrabber*)sg;
        AM_MEDIA_TYPE smt;
        ZeroMemory(&smt, sizeof(smt));
        smt.majortype = MEDIATYPE_Video;
        smt.subtype = MEDIASUBTYPE_MMDXSHOW_RGB32;
        smt.bFixedSizeSamples = TRUE;
        smt.lSampleSize = 1;
        AM_MEDIA_TYPE outmt;
        ZeroMemory(&outmt, sizeof(outmt));
        LONG bufsz = 0;
        IMediaSample* psamp = NULL;
        LONG later = 0;
        wprintf(L"grabber GetConnectedMediaType: 0x%08X\n", ig->GetConnectedMediaType(&outmt));
        wprintf(L"grabber SetBufferSamples(TRUE): 0x%08X\n", ig->SetBufferSamples(TRUE));
        wprintf(L"grabber GetCurrentBuffer: 0x%08X\n", ig->GetCurrentBuffer(&bufsz, NULL));
        wprintf(L"grabber GetCurrentSample: 0x%08X\n", ig->GetCurrentSample(&psamp));
        wprintf(L"grabber SetOneShot(TRUE): 0x%08X\n", ig->SetOneShot(TRUE));
        wprintf(L"grabber SetMediaType(&mt): 0x%08X\n", ig->SetMediaType(&smt));
        wprintf(L"grabber IsSampleLater: 0x%08X\n", ig->IsSampleLater(&later));
        ((IUnknown*)sg)->Release();
    }
    if (grab) grab->Release();
    CoUninitialize();
    return 0;
}
