#include <windows.h>
#include <dshow.h>

#include <cstdio>

namespace {
const wchar_t* g_stage = L"startup";

LONG WINAPI CrashHandler(EXCEPTION_POINTERS* info)
{
    fwprintf(stderr, L"CRASH stage=%ls code=0x%08lX address=%p\n", g_stage,
             info->ExceptionRecord->ExceptionCode,
             info->ExceptionRecord->ExceptionAddress);
    fflush(stderr);
    return EXCEPTION_EXECUTE_HANDLER;
}

const CLSID kEncoder = {
    0xD79D43B2, 0xF005, 0x40A4,
    {0xBE, 0x18, 0xAF, 0xD1, 0x9C, 0x03, 0xE6, 0xE6}};

using DllGetClassObjectFn = HRESULT(STDAPICALLTYPE*)(REFCLSID, REFIID, void**);
using DllCanUnloadNowFn = HRESULT(STDAPICALLTYPE*)();

bool CheckPin(IPin* pin, PIN_DIRECTION expected)
{
    PIN_DIRECTION direction = static_cast<PIN_DIRECTION>(-1);
    if (pin->QueryDirection(&direction) != S_OK || direction != expected) {
        return false;
    }

    PIN_INFO info = {};
    if (pin->QueryPinInfo(&info) != S_OK || info.dir != expected || !info.pFilter) {
        return false;
    }
    info.pFilter->Release();

    IPin* peer = reinterpret_cast<IPin*>(static_cast<uintptr_t>(0x1));
    const HRESULT connected = pin->ConnectedTo(&peer);
    if (connected != VFW_E_NOT_CONNECTED || peer != nullptr) {
        if (peer && peer != reinterpret_cast<IPin*>(static_cast<uintptr_t>(0x1))) {
            peer->Release();
        }
        return false;
    }
    return true;
}
}  // namespace

int wmain(int argc, wchar_t** argv)
{
    setvbuf(stdout, nullptr, _IONBF, 0);
    setvbuf(stderr, nullptr, _IONBF, 0);
    SetUnhandledExceptionFilter(CrashHandler);
    if (argc != 2) {
        fwprintf(stderr, L"usage: abi_smoke.exe <FFmpegVideoEncoder.dll>\n");
        return 2;
    }
    const HRESULT init = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    if (FAILED(init)) {
        return 3;
    }

    HMODULE module = LoadLibraryExW(argv[1], nullptr, LOAD_WITH_ALTERED_SEARCH_PATH);
    if (!module) {
        fwprintf(stderr, L"LoadLibraryEx failed: %lu\n", GetLastError());
        CoUninitialize();
        return 4;
    }
    auto getClass = reinterpret_cast<DllGetClassObjectFn>(
        GetProcAddress(module, "DllGetClassObject"));
    auto canUnload = reinterpret_cast<DllCanUnloadNowFn>(
        GetProcAddress(module, "DllCanUnloadNow"));
    if (!getClass || !canUnload) {
        FreeLibrary(module);
        CoUninitialize();
        return 5;
    }
    if (getClass(kEncoder, IID_IClassFactory, nullptr) != E_POINTER ||
        canUnload() != S_OK) {
        return 6;
    }

    IClassFactory* lockFactory = nullptr;
    if (FAILED(getClass(kEncoder, IID_IClassFactory,
                        reinterpret_cast<void**>(&lockFactory))) || !lockFactory) {
        return 7;
    }
    lockFactory->LockServer(TRUE);
    if (canUnload() != S_FALSE) {
        return 8;
    }
    lockFactory->LockServer(FALSE);
    lockFactory->Release();
    if (canUnload() != S_OK) {
        return 9;
    }

    for (int cycle = 0; cycle < 3000; ++cycle) {
        g_stage = L"DllGetClassObject";
        IClassFactory* factory = nullptr;
        HRESULT hr = getClass(kEncoder, IID_IClassFactory,
                              reinterpret_cast<void**>(&factory));
        if (FAILED(hr) || !factory) {
            return 10;
        }
        g_stage = L"CreateInstance";
        IBaseFilter* filter = nullptr;
        hr = factory->CreateInstance(nullptr, IID_IBaseFilter,
                                     reinterpret_cast<void**>(&filter));
        factory->Release();
        if (FAILED(hr) || !filter) {
            return 11;
        }

        g_stage = L"EnumPins";
        IEnumPins* pins = nullptr;
        hr = filter->EnumPins(&pins);
        if (FAILED(hr) || !pins) {
            filter->Release();
            return 12;
        }
        IPin* pin = nullptr;
        ULONG fetched = 0;
        int count = 0;
        while (pins->Next(1, &pin, &fetched) == S_OK) {
            g_stage = count == 0 ? L"check input pin" : L"check output pin";
            const auto direction = count == 0 ? PINDIR_INPUT : PINDIR_OUTPUT;
            const bool ok = CheckPin(pin, direction);
            pin->Release();
            pin = nullptr;
            if (!ok) {
                pins->Release();
                filter->Release();
                return 13;
            }
            ++count;
        }
        g_stage = L"release enum";
        pins->Release();
        g_stage = L"release filter";
        filter->Release();
        if (count != 2) {
            return 14;
        }
        if (canUnload() != S_OK) {
            return 15;
        }
    }

    g_stage = L"FreeLibrary";
    wprintf(L"PASS: 3000 native COM create/enum/query/ConnectedTo/release cycles\n");
    FreeLibrary(module);
    CoUninitialize();
    return 0;
}
