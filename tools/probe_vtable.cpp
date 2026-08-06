#include <windows.h>
#include <dshow.h>
#include <stdio.h>

// Empirically verify what MMD expects at IBaseFilter vtable offset 0x50.
// MMD calls it with a single out-pointer and treats the result as IEnumPins.
static const GUID CLSID_MJPGCompressor = {0xB80AB0A0, 0x7416, 0x11D2, {0x9E,0xEB,0x00,0x60,0x08,0x03,0x9E,0x37}};
static const GUID CLSID_SampleGrabberLocal = {0xC1F400A0, 0x3F08, 0x11D3, {0x9F,0x0B,0x00,0x60,0x08,0x03,0x9E,0x37}};

typedef HRESULT (STDMETHODCALLTYPE *PFN_1OUT)(void*, void**);
typedef HRESULT (STDMETHODCALLTYPE *PFN_NEXT)(void*, ULONG, void**, ULONG*);

static void TrySlot(IBaseFilter* f, const WCHAR* name)
{
    void** vt = *(void***)f;
    IEnumPins* e48 = NULL;
    HRESULT hr48 = ((PFN_1OUT)vt[0x48/8])(f, (void**)&e48);
    wprintf(L"%s: +0x48 => hr=0x%08X enum=%p\n", name, hr48, e48);
    if (SUCCEEDED(hr48) && e48) {
        IPin* pin = NULL;
        ULONG got = 0;
        HRESULT hn = e48->Next(1, &pin, &got);
        wprintf(L"   enum->Next hr=0x%08X pin=%p got=%u\n", hn, pin, got);
        if (pin) pin->Release();
        e48->Release();
    }

    void* out50 = (void*)0xDEAD;
    __try {
        HRESULT hr50 = ((PFN_1OUT)vt[0x50/8])(f, &out50);
        wprintf(L"%s: +0x50 => hr=0x%08X out=%p\n", name, hr50, out50);
        if (SUCCEEDED(hr50) && out50 && out50 != (void*)0xDEAD) {
            void** vt2 = *(void***)out50;
            ULONG got = 0;
            void* pin = NULL;
            __try {
                HRESULT hn = ((PFN_NEXT)vt2[0x18/8])(out50, 1, &pin, &got);
                wprintf(L"   out50->Next(1) hr=0x%08X pin=%p got=%u\n", hn, pin, got);
                if (pin) ((IUnknown*)pin)->Release();
            } __except(EXCEPTION_EXECUTE_HANDLER) {
                wprintf(L"   out50->Next crashed (not an IEnumPins?)\n");
            }
            __try {
                ((IUnknown*)out50)->Release();
            } __except(EXCEPTION_EXECUTE_HANDLER) {
                wprintf(L"   out50->Release crashed\n");
            }
        }
    } __except(EXCEPTION_EXECUTE_HANDLER) {
        wprintf(L"%s: +0x50 call crashed\n", name);
    }
}

int wmain()
{
    setvbuf(stdout, NULL, _IONBF, 0);
    CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);

    IBaseFilter* sg = NULL;
    HRESULT hr = CoCreateInstance(CLSID_SampleGrabberLocal, NULL, CLSCTX_INPROC_SERVER,
                                  IID_IBaseFilter, (void**)&sg);
    wprintf(L"SampleGrabber create: 0x%08X\n", hr);
    if (SUCCEEDED(hr)) { TrySlot(sg, L"SampleGrabber"); sg->Release(); }

    IBaseFilter* mj = NULL;
    hr = CoCreateInstance(CLSID_MJPGCompressor, NULL, CLSCTX_INPROC_SERVER,
                          IID_IBaseFilter, (void**)&mj);
    wprintf(L"MJPEG create: 0x%08X\n", hr);
    if (SUCCEEDED(hr)) { TrySlot(mj, L"MJPEG"); mj->Release(); }

    IBaseFilter* mux = NULL;
    hr = CoCreateInstance(CLSID_AviDest, NULL, CLSCTX_INPROC_SERVER,
                          IID_IBaseFilter, (void**)&mux);
    wprintf(L"AVI Mux create: 0x%08X\n", hr);
    if (SUCCEEDED(hr)) { TrySlot(mux, L"AVI Mux"); mux->Release(); }

    CoUninitialize();
    return 0;
}
