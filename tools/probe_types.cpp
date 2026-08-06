#include <windows.h>
#include <dshow.h>
#include <stdio.h>

static void FreeMT(AM_MEDIA_TYPE& mt)
{
    if (mt.pbFormat) { CoTaskMemFree(mt.pbFormat); mt.pbFormat = NULL; }
    ZeroMemory(&mt, sizeof(mt));
}

static void DeleteMT(AM_MEDIA_TYPE* mt)
{
    if (!mt) return;
    if (mt->pbFormat) CoTaskMemFree(mt->pbFormat);
    CoTaskMemFree(mt);
}

EXTERN_C const GUID CLSID_FFmpegEncoder = {
    0xD79D43B2, 0xF005, 0x40A4,
    { 0xBE, 0x18, 0xAF, 0xD1, 0x9C, 0x03, 0xE6, 0xE6 }
};
static const GUID CLSID_MJPGCompressor = {0xB80AB0A0, 0x7416, 0x11D2, {0x9E,0xEB,0x00,0x60,0x08,0x03,0x9E,0x37}};

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

static void DumpType(const AM_MEDIA_TYPE* mt, const WCHAR* label)
{
    wprintf(L"%s:\n", label);
    wprintf(L"  majortype=");
    if (mt->majortype == MEDIATYPE_Video) wprintf(L"Video");
    else wprintf(L"{%08X-%04X-%04X}", mt->majortype.Data1, mt->majortype.Data2, mt->majortype.Data3);
    wprintf(L" subtype={%08X-%04X-%04X} fixed=%d temporal=%d sampleSize=%u fmttype=",
            mt->subtype.Data1, mt->subtype.Data2, mt->subtype.Data3,
            mt->bFixedSizeSamples ? 1 : 0, mt->bTemporalCompression ? 1 : 0, mt->lSampleSize);
    if (mt->formattype == FORMAT_VideoInfo) wprintf(L"VideoInfo");
    else if (mt->formattype == FORMAT_VideoInfo2) wprintf(L"VideoInfo2");
    else wprintf(L"{%08X}", mt->formattype.Data1);
    wprintf(L" cbFormat=%u\n", mt->cbFormat);
    if (mt->pbFormat && mt->cbFormat >= sizeof(VIDEOINFOHEADER) && mt->formattype == FORMAT_VideoInfo) {
        const VIDEOINFOHEADER* vih = (const VIDEOINFOHEADER*)mt->pbFormat;
        const BITMAPINFOHEADER* bi = &vih->bmiHeader;
        wprintf(L"  rcSource=(%ld,%ld)-(%ld,%ld) AvgTimePerFrame=%I64d bitrate=%lu\n",
                vih->rcSource.left, vih->rcSource.top, vih->rcSource.right, vih->rcSource.bottom,
                vih->AvgTimePerFrame, vih->dwBitRate);
        wprintf(L"  biSize=%u biWidth=%ld biHeight=%ld biPlanes=%u biBitCount=%u biCompression=0x%08X '%c%c%c%c' biSizeImage=%lu\n",
                bi->biSize, bi->biWidth, bi->biHeight, bi->biPlanes, bi->biBitCount,
                bi->biCompression,
                (bi->biCompression >> 0) & 0xFF, (bi->biCompression >> 8) & 0xFF,
                (bi->biCompression >> 16) & 0xFF, (bi->biCompression >> 24) & 0xFF,
                bi->biSizeImage);
    }
}

static HRESULT ConnectAll(IGraphBuilder* pGraph, IBaseFilter* pA, IBaseFilter* pB)
{
    for (int i = 0;; i++) {
        IPin* pOut = GetPin(pA, PINDIR_OUTPUT, i);
        if (!pOut) break;
        for (int j = 0;; j++) {
            IPin* pIn = GetPin(pB, PINDIR_INPUT, j);
            if (!pIn) break;
            HRESULT hr = pGraph->Connect(pOut, pIn);
            pIn->Release();
            if (SUCCEEDED(hr)) { pOut->Release(); return hr; }
        }
        pOut->Release();
    }
    return VFW_E_CANNOT_CONNECT;
}

static void Run(const WCHAR* outlabel, REFCLSID encClsid)
{
    wprintf(L"=== %s ===\n", outlabel);
    CoInitializeEx(NULL, COINIT_APARTMENTTHREADED);
    IGraphBuilder* g = NULL;
    IBaseFilter* src = NULL, *split = NULL, *enc = NULL;
    HRESULT hr = CoCreateInstance(CLSID_FilterGraph, NULL, CLSCTX_INPROC_SERVER, IID_IGraphBuilder, (void**)&g);
    if (FAILED(hr)) { wprintf(L"graph: 0x%08X\n", hr); goto done; }
    hr = CoCreateInstance(CLSID_AsyncReader, NULL, CLSCTX_INPROC_SERVER, IID_IBaseFilter, (void**)&src);
    hr |= g->AddFilter(src, L"File Source");
    hr |= CoCreateInstance(CLSID_AviSplitter, NULL, CLSCTX_INPROC_SERVER, IID_IBaseFilter, (void**)&split);
    hr |= g->AddFilter(split, L"Splitter");
    hr |= CoCreateInstance(encClsid, NULL, CLSCTX_INPROC_SERVER, IID_IBaseFilter, (void**)&enc);
    hr |= g->AddFilter(enc, outlabel);
    if (FAILED(hr)) { wprintf(L"setup: 0x%08X\n", hr); goto done; }
    IFileSourceFilter* fs = NULL;
    src->QueryInterface(IID_IFileSourceFilter, (void**)&fs);
    hr = fs->Load(L"test_in.avi", NULL);
    wprintf(L"load: 0x%08X\n", hr);
    hr = ConnectAll(g, src, split);
    wprintf(L"src->split: 0x%08X\n", hr);
    hr = ConnectAll(g, split, enc);
    wprintf(L"split->enc: 0x%08X\n", hr);

    IPin* pOut = GetPin(enc, PINDIR_OUTPUT, 0);
    if (pOut) {
        IEnumMediaTypes* emt = NULL;
        hr = pOut->EnumMediaTypes(&emt);
        wprintf(L"output EnumMediaTypes: 0x%08X\n", hr);
        if (emt) {
            AM_MEDIA_TYPE* pmt = NULL;
            int n = 0;
            while (emt->Next(1, &pmt, NULL) == S_OK && n < 8) {
                WCHAR lbl[64];
                swprintf_s(lbl, L"  type[%d]", n++);
                DumpType(pmt, lbl);
                DeleteMT(pmt);
                pmt = NULL;
            }
            emt->Release();
        }
        pOut->Release();
    }
    IPin* pIn = GetPin(enc, PINDIR_INPUT, 0);
    if (pIn) {
        AM_MEDIA_TYPE mt;
        if (SUCCEEDED(pIn->ConnectionMediaType(&mt))) {
            DumpType(&mt, L"input connected type");
            FreeMT(mt);
        }
        pIn->Release();
    }
done:
    if (fs) fs->Release();
    if (src) src->Release();
    if (split) split->Release();
    if (enc) enc->Release();
    if (g) g->Release();
    CoUninitialize();
}

int wmain()
{
    setvbuf(stdout, NULL, _IONBF, 0);
    Run(L"FFmpegEncoder", CLSID_FFmpegEncoder);
    Run(L"MJPEG Compressor", CLSID_MJPGCompressor);
    return 0;
}
