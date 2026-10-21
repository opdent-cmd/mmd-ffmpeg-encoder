#include "rate_control.h"

#include <iostream>
#include <string>

static bool Contains(const std::wstring& value, const std::wstring& part)
{
    return value.find(part) != std::wstring::npos;
}

static bool Missing(const std::wstring& value, const std::wstring& part)
{
    return !Contains(value, part);
}

static int Fail(const wchar_t* label, const std::wstring& args)
{
    std::wcerr << L"FAIL " << label << L": " << args << L"\n";
    return 1;
}

int main()
{
    const long bitrate = 8'000'000;
    const std::wstring x264Vbr =
        rate_control::BuildArgs(L"libx264", bitrate, false, false);
    if (!Contains(x264Vbr, L"-b:v 8000000") ||
        !Contains(x264Vbr, L"-maxrate 12000000") ||
        !Contains(x264Vbr, L"-bufsize 16000000") ||
        Contains(x264Vbr, L"-minrate") || Contains(x264Vbr, L"-rc cbr")) {
        return Fail(L"x264 VBR bounds", x264Vbr);
    }

    const std::wstring nvencVbr =
        rate_control::BuildArgs(L"h264_nvenc", bitrate, false, false);
    if (!Contains(nvencVbr, L"-rc vbr") ||
        !Contains(nvencVbr, L"-maxrate 12000000")) {
        return Fail(L"NVENC VBR", nvencVbr);
    }

    const std::wstring amfVbr =
        rate_control::BuildArgs(L"h264_amf", bitrate, false, false);
    if (!Contains(amfVbr, L"-rc vbr_peak") ||
        !Contains(amfVbr, L"-maxrate 12000000")) {
        return Fail(L"AMF peak VBR", amfVbr);
    }

    const std::wstring qsvVbr =
        rate_control::BuildArgs(L"h264_qsv", bitrate, false, false);
    if (!Contains(qsvVbr, L"-maxrate 12000000") ||
        Contains(qsvVbr, L"-low_delay_brc") ||
        Contains(qsvVbr, L"-rc")) {
        return Fail(L"QSV VBR bounds", qsvVbr);
    }

    const std::wstring x265Vbr =
        rate_control::BuildArgs(L"libx265", bitrate, false, false);
    if (!Contains(x265Vbr, L"vbv-maxrate=12000") ||
        !Contains(x265Vbr, L"vbv-bufsize=16000")) {
        return Fail(L"x265 VBR bounds", x265Vbr);
    }

    const std::wstring x264Cbr =
        rate_control::BuildArgs(L"libx264", bitrate, true, false);
    if (!Contains(x264Cbr, L"-minrate 8000000") ||
        !Contains(x264Cbr, L"-maxrate 8000000") ||
        !Contains(x264Cbr, L"nal-hrd=cbr:filler=1")) {
        return Fail(L"x264 CBR", x264Cbr);
    }

    const std::wstring x265Cbr =
        rate_control::BuildArgs(L"libx265", bitrate, true, false);
    if (!Contains(x265Cbr, L"vbv-maxrate=8000") ||
        !Contains(x265Cbr, L"hrd=1:strict-cbr=1") ||
        Contains(x265Cbr, L"nal-hrd")) {
        return Fail(L"x265 CBR", x265Cbr);
    }

    const std::wstring nvencCbr =
        rate_control::BuildArgs(L"h264_nvenc", bitrate, true, true);
    if (!Contains(nvencCbr, L"-rc cbr") ||
        Contains(nvencCbr, L"-cbr 1") ||
        !Contains(nvencCbr, L"-cbr_padding 1") ||
        !Contains(nvencCbr, L"-maxrate 8000000")) {
        return Fail(L"NVENC CBR", nvencCbr);
    }

    const std::wstring qsvCbr =
        rate_control::BuildArgs(L"h264_qsv", bitrate, true, false);
    if (!Contains(qsvCbr, L"-low_delay_brc 1") ||
        Missing(qsvCbr, L"-maxrate 8000000")) {
        return Fail(L"QSV CBR", qsvCbr);
    }

    const std::wstring amfCbr =
        rate_control::BuildArgs(L"h264_amf", bitrate, true, false);
    if (!Contains(amfCbr, L"-rc cbr") ||
        !Contains(amfCbr, L"-enforce_hrd 1") ||
        !Contains(amfCbr, L"-filler_data 1")) {
        return Fail(L"AMF CBR", amfCbr);
    }

    if (!rate_control::BuildArgs(L"libx264", 0, true, false).empty()) {
        std::wcerr << L"FAIL zero bitrate must not emit rate-control args\n";
        return 1;
    }

    std::wcout << L"PASS VBR/CBR argument cases\n";
    return 0;
}
