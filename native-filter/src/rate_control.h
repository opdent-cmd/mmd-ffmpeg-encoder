#pragma once

#include <string>

namespace rate_control {

inline std::wstring BuildArgs(const std::wstring& codec, long bitrate,
                              bool cbr, bool nvencCbrPadding)
{
    if (bitrate <= 0) {
        return std::wstring();
    }

    const bool nvenc = codec.find(L"nvenc") != std::wstring::npos;
    const bool qsv = codec.find(L"qsv") != std::wstring::npos;
    const bool amf = codec.find(L"amf") != std::wstring::npos;
    const bool software = !nvenc && !qsv && !amf;
    const long maxrate = cbr ? bitrate : bitrate + bitrate / 2;
    const long bufferSize = bitrate * 2;
    std::wstring args = L" -b:v " + std::to_wstring(bitrate);

    if (cbr) {
        args += L" -minrate " + std::to_wstring(bitrate) +
                L" -maxrate " + std::to_wstring(maxrate) +
                L" -bufsize " + std::to_wstring(bufferSize);
        if (nvenc) {
            args += L" -rc cbr -cbr 1 -strict_gop 1";
            if (nvencCbrPadding) {
                args += L" -cbr_padding 1";
            }
        } else if (amf) {
            args += L" -rc cbr -enforce_hrd 1 -filler_data 1";
        } else if (qsv) {
            args += L" -low_delay_brc 1";
        }

        if (software && codec.find(L"264") != std::wstring::npos) {
            args += L" -x264-params nal-hrd=cbr:filler=1";
        } else if (software && (codec.find(L"265") != std::wstring::npos ||
                                codec.find(L"hevc") != std::wstring::npos)) {
            const long bitrateKbps = bitrate / 1000;
            const long maxrateKbps = maxrate / 1000;
            const long bufferKbps = bufferSize / 1000;
            args += L" -x265-params bitrate=" + std::to_wstring(bitrateKbps) +
                    L":vbv-maxrate=" + std::to_wstring(maxrateKbps) +
                    L":vbv-bufsize=" + std::to_wstring(bufferKbps) +
                    L":hrd=1:strict-cbr=1";
        }
    } else {
        args += L" -maxrate " + std::to_wstring(maxrate) +
                L" -bufsize " + std::to_wstring(bufferSize);
        if (nvenc) {
            args += L" -rc vbr";
        } else if (amf) {
            args += L" -rc vbr_peak";
        }
        if (software && (codec.find(L"265") != std::wstring::npos ||
                         codec.find(L"hevc") != std::wstring::npos)) {
            const long bitrateKbps = bitrate / 1000;
            const long maxrateKbps = maxrate / 1000;
            const long bufferKbps = bufferSize / 1000;
            args += L" -x265-params bitrate=" + std::to_wstring(bitrateKbps) +
                    L":vbv-maxrate=" + std::to_wstring(maxrateKbps) +
                    L":vbv-bufsize=" + std::to_wstring(bufferKbps);
        }
    }
    return args;
}

} // namespace rate_control
