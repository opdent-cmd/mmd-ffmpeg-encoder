#pragma once

#include <windows.h>
#include <streams.h>
#include <dvdmedia.h>
#include <string>
#include <vector>
#include <deque>

// CLSID: {D79D43B2-F005-40A4-BE18-AFD19C03E6E6}
DEFINE_GUID(CLSID_FFmpegEncoder,
    0xD79D43B2, 0xF005, 0x40A4, 0xBE, 0x18, 0xAF, 0xD1, 0x9C, 0x03, 0xE6, 0xE6);

// A DirectShow transform filter that feeds raw RGB frames to an ffmpeg child
// process and pushes the encoded elementary stream downstream (to AVI Mux).
class CFFmpegEncoder : public CTransformFilter
{
public:
    static CUnknown* WINAPI CreateInstance(LPUNKNOWN pUnk, HRESULT* phr);

    CFFmpegEncoder(LPUNKNOWN pUnk, HRESULT* phr);
    ~CFFmpegEncoder();

    // --- CTransformFilter overrides ---
    HRESULT CheckInputType(const CMediaType* mtIn);
    HRESULT CheckTransform(const CMediaType* mtIn, const CMediaType* mtOut);
    HRESULT GetMediaType(int iPosition, CMediaType* pmt);
    HRESULT SetMediaType(PIN_DIRECTION direction, const CMediaType* pmt);
    HRESULT DecideBufferSize(IMemAllocator* pAlloc, ALLOCATOR_PROPERTIES* pProps);
    HRESULT StartStreaming();
    HRESULT StopStreaming();
    HRESULT Transform(IMediaSample* pIn, IMediaSample* pOut);
    HRESULT Receive(IMediaSample* pSample);
    HRESULT EndOfStream();

private:
    struct Packet
    {
        std::vector<BYTE> data;
    };

    HRESULT ReadConfig();
    void    BuildCmdLine(std::wstring& cmd);
    HRESULT StartFFmpeg();
    void    StopFFmpeg();
    void    WriteFrame(const BYTE* data, long cb);
    void    DrainQueue();
    HRESULT DeliverPacket(Packet& pkt);

    static DWORD WINAPI ReaderThreadProc(LPVOID param);
    void ReaderLoop();
    void OnRead(const BYTE* data, size_t len);
    void ParseAnnexB();
    void FlushGroup(bool atEof);
    static bool NalIsVcl(const BYTE* nal, size_t len);
    static GUID FourccGuid(DWORD fcc);

    // Format state (parsed in SetMediaType)
    int     m_width = 0;
    int     m_height = 0;
    int     m_bpp = 0;             // bits per pixel of the input
    bool    m_bottomUp = false;    // DIB rows are stored bottom-up
    REFERENCE_TIME m_frameDur = 1; // AvgTimePerFrame, 100ns units
    GUID    m_inSubtype = GUID_NULL;
    std::wstring m_pixfmt;         // ffmpeg rawvideo pixel format

    // Output codec state (from config)
    DWORD   m_fourcc = 0;          // FOURCC handed to AVI Mux
    std::wstring m_outMux;         // ffmpeg elementary-stream muxer name
    std::wstring m_codec;          // -c:v value
    bool    m_lossless = false;
    bool    m_nvenc = false;

    // Config
    std::wstring m_ffmpegPath;
    std::wstring m_preset;
    std::wstring m_extra;
    int     m_crf = 18;
    long    m_bitrate = 0;

    // Child process / pipes
    HANDLE  m_hProc = NULL;
    HANDLE  m_hChildStdinW = NULL;  // our write end (child stdin)
    HANDLE  m_hChildStdoutR = NULL; // our read end (child stdout)
    HANDLE  m_hThread = NULL;
    volatile bool m_readerDone = false;

    // Shared reader/stream state
    CCritSec m_lock;
    std::vector<BYTE> m_inBuf;      // Annex B byte accumulation
    std::vector<BYTE> m_group;      // current access unit being assembled
    bool    m_groupHasVcl = false;
    std::deque<Packet> m_pktQueue;
    std::deque<REFERENCE_TIME> m_tsQueue;
    REFERENCE_TIME m_lastTs = 0;
    bool    m_firstPkt = true;
    bool    m_started = false;
};
