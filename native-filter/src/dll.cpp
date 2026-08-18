#include <windows.h>
#include <streams.h>
#include <stdio.h>
#include <wchar.h>
#include "ffmpeg_encoder.h"

HINSTANCE g_hInst = NULL;
static LONG g_serverLocks = 0;

// Definition of our CLSID (the header only declares it via DEFINE_GUID).
EXTERN_C const GUID CLSID_FFmpegEncoder = {
    0xD79D43B2, 0xF005, 0x40A4,
    { 0xBE, 0x18, 0xAF, 0xD1, 0x9C, 0x03, 0xE6, 0xE6 }
};

CFactoryTemplate g_Templates[] = {
    { L"FFmpeg Video Encoder (H.264/HEVC/AV1)",
      &CLSID_FFmpegEncoder,
      CFFmpegEncoder::CreateInstance,
      NULL,
      NULL }
};
int g_cTemplates = sizeof(g_Templates) / sizeof(g_Templates[0]);

class CClassFactory : public IClassFactory
{
public:
    CClassFactory(const CFactoryTemplate* pTemplate)
        : m_cRef(1), m_pTemplate(pTemplate)
    {
    }

    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override
    {
        if (!ppv) {
            return E_POINTER;
        }
        *ppv = NULL;
        if (riid == IID_IUnknown || riid == IID_IClassFactory) {
            *ppv = (IClassFactory*)this;
            AddRef();
            return S_OK;
        }
        return E_NOINTERFACE;
    }

    STDMETHODIMP_(ULONG) AddRef() override
    {
        return InterlockedIncrement(&m_cRef);
    }

    STDMETHODIMP_(ULONG) Release() override
    {
        LONG r = InterlockedDecrement(&m_cRef);
        if (r == 0) {
            delete this;
        }
        return r;
    }

    STDMETHODIMP CreateInstance(LPUNKNOWN pUnkOuter, REFIID riid, void** ppv) override
    {
        if (!ppv) {
            return E_POINTER;
        }
        *ppv = NULL;
        if (pUnkOuter) {
            return CLASS_E_NOAGGREGATION;
        }
        HRESULT hr = S_OK;
        CUnknown* pUnk = m_pTemplate->CreateInstance(NULL, &hr);
        if (!pUnk) {
            return hr;
        }
        // BaseClasses objects start with a zero reference count. Hold a
        // construction reference across QI; otherwise releasing here destroys
        // the object while returning a dangling interface pointer to MMD.
        pUnk->NonDelegatingAddRef();
        hr = pUnk->NonDelegatingQueryInterface(riid, ppv);
        pUnk->NonDelegatingRelease();
        return hr;
    }

    STDMETHODIMP LockServer(BOOL fLock) override
    {
        if (fLock) {
            InterlockedIncrement(&g_serverLocks);
        } else {
            InterlockedDecrement(&g_serverLocks);
        }
        return S_OK;
    }

private:
    LONG m_cRef;
    const CFactoryTemplate* m_pTemplate;
};

extern "C" BOOL WINAPI DllMain(HINSTANCE hInstance, DWORD dwReason, LPVOID /*lpReserved*/)
{
    if (dwReason == DLL_PROCESS_ATTACH) {
        g_hInst = hInstance;
        DisableThreadLibraryCalls(hInstance);
        DbgInitialise(hInstance);
    } else if (dwReason == DLL_PROCESS_DETACH) {
        DbgTerminate();
    }
    return TRUE;
}

STDAPI DllGetClassObject(REFCLSID rclsid, REFIID riid, void** ppv)
{
    if (!ppv) {
        return E_POINTER;
    }
    *ppv = NULL;
    if (!g_cTemplates) {
        return E_OUTOFMEMORY;
    }
    for (int i = 0; i < g_cTemplates; i++) {
        if (g_Templates[i].IsClassID(rclsid)) {
            CClassFactory* pCF = new CClassFactory(&g_Templates[i]);
            if (!pCF) {
                return E_OUTOFMEMORY;
            }
            HRESULT hr = pCF->QueryInterface(riid, ppv);
            pCF->Release();
            return hr;
        }
    }
    return CLASS_E_CLASSNOTAVAILABLE;
}

STDAPI DllCanUnloadNow(void)
{
    if (CBaseObject::ObjectsActive() ||
        InterlockedCompareExchange(&g_serverLocks, 0, 0) != 0) {
        return S_FALSE;
    }
    return S_OK;
}

// ---------------------------------------------------------------------------
// Registration: also register under the Video Compressor and DirectShow
// Filters categories so MMD's AVI-out dialog can see this encoder.
// ---------------------------------------------------------------------------

namespace {
    const WCHAR kClsid[] = L"{D79D43B2-F005-40A4-BE18-AFD19C03E6E6}";
    const WCHAR kFriendlyName[] = L"FFmpeg Video Encoder (H.264/HEVC/AV1)";
    const WCHAR kCategories[][40] = {
        L"{33d9a760-90c8-11d0-bd43-00a0c911ce86}",  // Video Compressor
        L"{860bb310-5d01-11d0-bd3b-00a0c911ce86}",  // DirectShow Filters
    };

    HRESULT RegisterCategories(BOOL bRegister)
    {
        for (int i = 0; i < 2; i++) {
            WCHAR sub[256];
            swprintf_s(sub, L"CLSID\\%s\\Instance\\%s", kCategories[i], kClsid);
            if (bRegister) {
                HKEY hk = NULL;
                LONG r = RegCreateKeyExW(HKEY_CLASSES_ROOT, sub, 0, NULL,
                                         REG_OPTION_NON_VOLATILE, KEY_WRITE,
                                         NULL, &hk, NULL);
                if (r != ERROR_SUCCESS) {
                    return E_FAIL;
                }
                RegSetValueExW(hk, L"CLSID", 0, REG_SZ,
                               (const BYTE*)kClsid, (DWORD)(sizeof(kClsid)));
                RegSetValueExW(hk, L"FriendlyName", 0, REG_SZ,
                               (const BYTE*)kFriendlyName,
                               (DWORD)(sizeof(kFriendlyName)));
                RegCloseKey(hk);
            } else {
                RegDeleteTreeW(HKEY_CLASSES_ROOT, sub);
            }
        }
        return S_OK;
    }
}

STDAPI DllRegisterServer(void)
{
    HRESULT hr = AMovieDllRegisterServer2(TRUE);
    if (FAILED(hr)) {
        return hr;
    }
    return RegisterCategories(TRUE);
}

STDAPI DllUnregisterServer(void)
{
    RegisterCategories(FALSE);
    return AMovieDllRegisterServer2(FALSE);
}
