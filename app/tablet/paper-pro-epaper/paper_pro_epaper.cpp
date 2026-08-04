#include "paper_pro_epaper.h"

#include <QCoreApplication>
#include <QEventLoop>
#include <QImage>
#include <QRect>

#include <algorithm>
#include <atomic>
#include <cerrno>
#include <climits>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <dlfcn.h>
#include <limits>
#include <mutex>
#include <vector>

namespace {

using ImageCleanup = void (*)(void *);
using ImageConstructor = void (*)(QImage *, unsigned char *, int, int, qint64,
                                  QImage::Format, ImageCleanup, void *);

struct FramebufferCandidate {
    QImage *image;
    unsigned char *pixels;
    int width;
    int height;
    qsizetype stride;
    QImage::Format format;
    ImageCleanup cleanup;
};

struct FramebufferView {
    unsigned char *pixels = nullptr;
    int width = 0;
    int height = 0;
    qsizetype stride = 0;
    QImage::Format format = QImage::Format_Invalid;
};

struct UpdateRectangle {
    int x;
    int y;
    int width;
    int height;
};

using SubmitUpdate = void (*)(void *, QRect, int, int, int);

std::atomic<bool> observing_framebuffer_construction{false};
std::mutex framebuffer_candidates_mutex;
std::vector<FramebufferCandidate> framebuffer_candidates;
std::mutex constructor_resolution_mutex;
std::atomic<ImageConstructor> real_qimage_c1{nullptr};
std::atomic<ImageConstructor> real_qimage_c2{nullptr};
thread_local bool resolving_qimage_constructor = false;

std::mutex open_mutex;
QCoreApplication *qt_application = nullptr;
void *epaper_library = nullptr;
void *vendor_framebuffer = nullptr;
SubmitUpdate vendor_submit_update = nullptr;
FramebufferView framebuffer;
bool epaper_is_open = false;
int open_error = 0;

ImageConstructor resolve_qimage_constructor(
    const char *symbol, std::atomic<ImageConstructor> &cached_constructor) {
    ImageConstructor constructor =
        cached_constructor.load(std::memory_order_acquire);
    if (constructor) return constructor;

    std::lock_guard<std::mutex> lock(constructor_resolution_mutex);
    constructor = cached_constructor.load(std::memory_order_relaxed);
    if (constructor) return constructor;
    if (resolving_qimage_constructor) {
        std::fputs("remarque epaper: recursive QImage constructor resolution\n",
                   stderr);
        std::abort();
    }

    resolving_qimage_constructor = true;
    dlerror();
    constructor =
        reinterpret_cast<ImageConstructor>(dlsym(RTLD_NEXT, symbol));
    const char *error = dlerror();
    resolving_qimage_constructor = false;
    if (!constructor) {
        std::fprintf(stderr, "remarque epaper: cannot resolve %s: %s\n",
                     symbol, error ? error : "unknown error");
        std::abort();
    }
    cached_constructor.store(constructor, std::memory_order_release);
    return constructor;
}

void record_framebuffer_candidate(QImage *image, unsigned char *pixels,
                                  ImageCleanup cleanup) {
    if (!observing_framebuffer_construction.load(std::memory_order_acquire))
        return;
    FramebufferCandidate candidate{image,
                                   pixels,
                                   image->width(),
                                   image->height(),
                                   image->bytesPerLine(),
                                   image->format(),
                                   cleanup};
    std::lock_guard<std::mutex> lock(framebuffer_candidates_mutex);
    framebuffer_candidates.push_back(candidate);
}

void call_qimage_constructor_and_record(
    const char *symbol, std::atomic<ImageConstructor> &cached_constructor,
    QImage *image, unsigned char *pixels, int width, int height, qint64 stride,
    QImage::Format format, ImageCleanup cleanup, void *cleanup_context) {
    resolve_qimage_constructor(symbol, cached_constructor)(
        image, pixels, width, height, stride, format, cleanup, cleanup_context);
    record_framebuffer_candidate(image, pixels, cleanup);
}

bool is_plausible_framebuffer(const FramebufferCandidate &candidate) {
    if (!candidate.image || !candidate.pixels || candidate.width <= 0 ||
        candidate.height <= 0 || candidate.stride <= 0)
        return false;
    if (candidate.format != QImage::Format_RGB32) return false;
    const qint64 minimum_stride = static_cast<qint64>(candidate.width) * 4;
    return candidate.stride >= minimum_stride && candidate.stride <= INT_MAX;
}

int configured_framebuffer_index() {
    const char *text = std::getenv("REMARQUE_EPAPER_FRAMEBUFFER_INDEX");
    if (!text || !*text) return -1;
    char *end = nullptr;
    errno = 0;
    const long value = std::strtol(text, &end, 10);
    if (errno || *end || value < 0 || value > INT_MAX) return -2;
    return static_cast<int>(value);
}

bool clip_update_rectangle(int x, int y, int width, int height,
                           UpdateRectangle *result) {
    if (!result || width <= 0 || height <= 0 || framebuffer.width <= 0 ||
        framebuffer.height <= 0)
        return false;
    const int64_t right = int64_t(x) + int64_t(width);
    const int64_t bottom = int64_t(y) + int64_t(height);
    const int64_t x0 = std::max<int64_t>(0, x);
    const int64_t y0 = std::max<int64_t>(0, y);
    const int64_t x1 = std::min<int64_t>(framebuffer.width, right);
    const int64_t y1 = std::min<int64_t>(framebuffer.height, bottom);
    if (x1 <= x0 || y1 <= y0) return false;
    *result = {int(x0), int(y0), int(x1 - x0), int(y1 - y0)};
    return true;
}

int capture_vendor_framebuffer() {
    epaper_library =
        dlopen("libqsgepaper.so", RTLD_NOW | RTLD_GLOBAL);
    if (!epaper_library) {
        const char *error = dlerror();
        std::fprintf(stderr, "remarque epaper: cannot load libqsgepaper.so: %s\n",
                     error ? error : "unknown error");
        return 2;
    }

    using FramebufferInstance = void *(*)();
    dlerror();
    auto framebuffer_instance = reinterpret_cast<FramebufferInstance>(dlsym(
        epaper_library, "_ZN13EPFramebuffer8instanceEv"));
    if (!framebuffer_instance) {
        const char *error = dlerror();
        std::fprintf(stderr,
                     "remarque epaper: EPFramebuffer::instance unavailable: %s\n",
                     error ? error : "unknown error");
        return 3;
    }
    dlerror();
    vendor_submit_update = reinterpret_cast<SubmitUpdate>(dlsym(
        epaper_library,
        "_ZN13EPFramebuffer11swapBuffersE5QRect13EPContentType12EPScreenMode6QFlagsINS_10UpdateFlagEE"));
    if (!vendor_submit_update) {
        const char *error = dlerror();
        std::fprintf(
            stderr,
            "remarque epaper: EPFramebuffer::swapBuffers unavailable: %s\n",
            error ? error : "unknown error");
        return 8;
    }

    {
        std::lock_guard<std::mutex> lock(framebuffer_candidates_mutex);
        framebuffer_candidates.clear();
    }
    observing_framebuffer_construction.store(true, std::memory_order_release);
    vendor_framebuffer = framebuffer_instance();
    observing_framebuffer_construction.store(false, std::memory_order_release);
    if (!vendor_framebuffer) return 4;

    std::vector<FramebufferCandidate> valid_candidates;
    {
        std::lock_guard<std::mutex> lock(framebuffer_candidates_mutex);
        for (const FramebufferCandidate &candidate : framebuffer_candidates) {
            if (!is_plausible_framebuffer(candidate)) continue;
            const bool duplicate = std::any_of(
                valid_candidates.begin(), valid_candidates.end(),
                [&candidate](const FramebufferCandidate &existing) {
                    return existing.image == candidate.image &&
                           existing.pixels == candidate.pixels;
                });
            if (!duplicate) valid_candidates.push_back(candidate);
        }
    }
    if (valid_candidates.empty()) return 5;

    const int requested_index = configured_framebuffer_index();
    if (requested_index == -2 ||
        (requested_index >= 0 &&
         requested_index >= static_cast<int>(valid_candidates.size()))) {
        std::fputs("remarque epaper: invalid REMARQUE_EPAPER_FRAMEBUFFER_INDEX\n",
                   stderr);
        return 6;
    }
    if (requested_index < 0 && valid_candidates.size() != 1) {
        std::fprintf(stderr,
                     "remarque epaper: %zu plausible framebuffer images\n",
                     valid_candidates.size());
        for (size_t index = 0; index < valid_candidates.size(); ++index) {
            const FramebufferCandidate &candidate = valid_candidates[index];
            std::fprintf(stderr,
                         "remarque epaper: candidate %zu: %dx%d stride=%lld "
                         "format=%d cleanup=%s\n",
                         index, candidate.width, candidate.height,
                         static_cast<long long>(candidate.stride),
                         int(candidate.format),
                         candidate.cleanup ? "yes" : "no");
        }
        std::fputs(
            "remarque epaper: validate the device, then set "
            "REMARQUE_EPAPER_FRAMEBUFFER_INDEX\n",
            stderr);
        return 6;
    }

    const FramebufferCandidate &selected =
        valid_candidates[requested_index < 0 ? 0 : requested_index];
    framebuffer = {selected.pixels, selected.width, selected.height,
                   selected.stride, selected.format};
    return 0;
}

}  // namespace

extern "C" void qimage_external_c1(
    QImage *image, unsigned char *pixels, int width, int height, qint64 stride,
    QImage::Format format, ImageCleanup cleanup,
    void *cleanup_context) asm("_ZN6QImageC1EPhiixNS_6FormatEPFvPvES2_");
extern "C" void qimage_external_c1(
    QImage *image, unsigned char *pixels, int width, int height, qint64 stride,
    QImage::Format format, ImageCleanup cleanup, void *cleanup_context) {
    call_qimage_constructor_and_record(
        "_ZN6QImageC1EPhiixNS_6FormatEPFvPvES2_", real_qimage_c1, image,
        pixels, width, height, stride, format, cleanup, cleanup_context);
}

extern "C" void qimage_external_c2(
    QImage *image, unsigned char *pixels, int width, int height, qint64 stride,
    QImage::Format format, ImageCleanup cleanup,
    void *cleanup_context) asm("_ZN6QImageC2EPhiixNS_6FormatEPFvPvES2_");
extern "C" void qimage_external_c2(
    QImage *image, unsigned char *pixels, int width, int height, qint64 stride,
    QImage::Format format, ImageCleanup cleanup, void *cleanup_context) {
    call_qimage_constructor_and_record(
        "_ZN6QImageC2EPhiixNS_6FormatEPFvPvES2_", real_qimage_c2, image,
        pixels, width, height, stride, format, cleanup, cleanup_context);
}

extern "C" {

int paper_pro_epaper_open(PaperProEpaperFramebuffer *result) {
    if (!result) return 7;
    std::lock_guard<std::mutex> lock(open_mutex);
    if (epaper_is_open) {
        *result = {framebuffer.pixels, framebuffer.width, framebuffer.height,
                   int(framebuffer.stride), int(framebuffer.format)};
        return 0;
    }
    if (open_error) return open_error;

    qt_application = QCoreApplication::instance();
    if (!qt_application) {
        static int argument_count = 1;
        static char application_name[] = "remarque";
        static char *arguments[] = {application_name, nullptr};
        qt_application = new QCoreApplication(argument_count, arguments);
    }
    if (!qt_application) return open_error = 1;

    open_error = capture_vendor_framebuffer();
    if (open_error) return open_error;
    if (!framebuffer.pixels || framebuffer.width <= 0 ||
        framebuffer.height <= 0 || framebuffer.stride <= 0 ||
        framebuffer.stride > std::numeric_limits<int>::max())
        return open_error = 7;

    epaper_is_open = true;
    *result = {framebuffer.pixels, framebuffer.width, framebuffer.height,
               int(framebuffer.stride), int(framebuffer.format)};
    std::fprintf(stderr,
                 "remarque epaper: framebuffer %dx%d stride=%lld format=%d\n",
                 framebuffer.width, framebuffer.height,
                 static_cast<long long>(framebuffer.stride),
                 int(framebuffer.format));
    return 0;
}

int paper_pro_epaper_submit_update(int32_t x, int32_t y, int32_t width,
                                   int32_t height, int32_t content_type,
                                   int32_t screen_mode,
                                   int32_t update_flags) {
    if (!epaper_is_open ||
        (content_type != 0 && content_type != 1))
        return 0;
    UpdateRectangle rectangle{};
    if (!clip_update_rectangle(x, y, width, height, &rectangle)) return 0;

    if (!vendor_submit_update || !vendor_framebuffer) return 0;
    vendor_submit_update(vendor_framebuffer,
                         QRect(rectangle.x, rectangle.y, rectangle.width,
                               rectangle.height),
                         content_type, screen_mode, update_flags);
    return 1;
}

int paper_pro_epaper_wait_until_update_queue_empty(void) {
    using WaitUntilEmpty = void (*)(void *);
    static WaitUntilEmpty wait_until_empty = [] {
        auto function = reinterpret_cast<WaitUntilEmpty>(dlsym(
            epaper_library, "_ZN19EPFramebufferSwtcon4syncEv"));
        if (!function)
            std::fputs(
                "remarque epaper: EPFramebufferSwtcon::sync unavailable\n",
                stderr);
        return function;
    }();
    if (!epaper_is_open || !wait_until_empty || !vendor_framebuffer) return 0;
    wait_until_empty(vendor_framebuffer);
    return 1;
}

void paper_pro_epaper_run_pending_events(void) {
    if (epaper_is_open && qt_application)
        QCoreApplication::processEvents(QEventLoop::AllEvents, 0);
}

}  // extern "C"
