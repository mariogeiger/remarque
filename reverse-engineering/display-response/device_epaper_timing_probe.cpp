#include "paper_pro_epaper.h"

#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <iterator>
#include <unistd.h>

namespace {

struct Update {
    const char *name;
    unsigned char blue;
    unsigned char green;
    unsigned char red;
    int content_type;
    int screen_mode;
};

uint64_t monotonic_microseconds() {
    timespec now{};
    clock_gettime(CLOCK_MONOTONIC, &now);
    return uint64_t(now.tv_sec) * 1'000'000 + uint64_t(now.tv_nsec) / 1'000;
}

void fill_rectangle(unsigned char *pixels, int stride, int x, int y,
                    int width, int height, const Update &update) {
    for (int row = y; row < y + height; ++row) {
        unsigned char *target =
            pixels + size_t(row) * stride + size_t(x) * 4;
        for (int column = 0; column < width; ++column) {
            target[column * 4 + 0] = update.blue;
            target[column * 4 + 1] = update.green;
            target[column * 4 + 2] = update.red;
            target[column * 4 + 3] = 0xff;
        }
    }
}

bool submit_and_wait(const Update &update, PaperProEpaperFramebuffer framebuffer,
                     int x, int y, int size, int repetition,
                     const char *direction) {
    fill_rectangle(framebuffer.pixels, framebuffer.stride, x, y, size, size,
                   update);
    const uint64_t before_submit = monotonic_microseconds();
    const int accepted = paper_pro_epaper_submit_update(
        x, y, size, size, update.content_type, update.screen_mode, 0);
    const uint64_t after_submit = monotonic_microseconds();
    const int drained = paper_pro_epaper_wait_until_update_queue_empty();
    const uint64_t after_drain = monotonic_microseconds();
    std::printf("%s,%s,%dx%d,%d,%llu,%llu,%llu,%d,%d\n", update.name,
                direction, size, size, repetition,
                static_cast<unsigned long long>(before_submit),
                static_cast<unsigned long long>(after_submit),
                static_cast<unsigned long long>(after_drain), accepted, drained);
    std::fflush(stdout);
    usleep(500'000);
    return accepted == 1 && drained == 1;
}

}  // namespace

int main(int argument_count, char **arguments) {
    const int repetitions = argument_count > 1 ? std::atoi(arguments[1]) : 2;
    if (repetitions <= 0 || repetitions > 20) return 2;
    const char *operation = argument_count > 2 ? arguments[2] : "all";
    const bool calibration = std::strcmp(operation, "calibrate") == 0;
    if (argument_count > 3 ||
        (std::strcmp(operation, "all") != 0 && !calibration &&
         std::strcmp(operation, "mode0") != 0 &&
         std::strcmp(operation, "mode3") != 0 &&
         std::strcmp(operation, "mode4") != 0))
        return 2;

    PaperProEpaperFramebuffer framebuffer{};
    if (paper_pro_epaper_open(&framebuffer) != 0) return 3;
    if (!framebuffer.pixels || framebuffer.width <= 0 ||
        framebuffer.height <= 0 ||
        framebuffer.stride < framebuffer.width * 4)
        return 4;

    const size_t byte_count = size_t(framebuffer.stride) * framebuffer.height;
    auto *saved = static_cast<unsigned char *>(std::malloc(byte_count));
    if (!saved) return 5;
    std::memcpy(saved, framebuffer.pixels, byte_count);

    constexpr Update black[] = {
        {"mono-fast-black", 0, 0, 0, 0, 0},
        {"color3-black", 0, 0, 0, 1, 3},
        {"color-black", 0, 0, 0, 1, 4},
    };
    constexpr Update paper[] = {
        {"mono-fast-paper", 235, 235, 235, 0, 0},
        {"color3-paper", 235, 235, 235, 1, 3},
        {"color-paper", 235, 235, 235, 1, 4},
    };
    constexpr int sizes[] = {64, 256, 512};
    const int center_x = framebuffer.width / 4;
    const int center_y = framebuffer.height / 2;

    if (calibration) {
        const int size = 512;
        fill_rectangle(framebuffer.pixels, framebuffer.stride,
                       center_x - size / 2, center_y - size / 2, size, size,
                       black[0]);
        const bool accepted = paper_pro_epaper_submit_update(
                                  center_x - size / 2, center_y - size / 2,
                                  size, size, 0, 0, 0) == 1;
        const bool drained =
            paper_pro_epaper_wait_until_update_queue_empty() == 1;
        sleep(10);
        std::memcpy(framebuffer.pixels, saved, byte_count);
        std::free(saved);
        const bool restored = paper_pro_epaper_submit_update(
                                  0, 0, framebuffer.width,
                                  framebuffer.height, 1, 4, 1) == 1 &&
                              paper_pro_epaper_wait_until_update_queue_empty() ==
                                  1;
        return accepted && drained && restored ? 0 : 6;
    }

    std::puts("update,direction,size,repetition,before_submit_us,after_submit_us,after_drain_us,accepted,drained");
    bool passed = true;
    size_t first_update = 0;
    size_t end_update = std::size(black);
    if (std::strcmp(operation, "mode0") == 0) end_update = 1;
    if (std::strcmp(operation, "mode3") == 0) {
        first_update = 1;
        end_update = 2;
    }
    if (std::strcmp(operation, "mode4") == 0) first_update = 2;
    for (size_t update_index = first_update; update_index < end_update;
         ++update_index) {
        for (const int size : sizes) {
            const int x = center_x - size / 2;
            const int y = center_y - size / 2;
            for (int repetition = 0; repetition < repetitions; ++repetition) {
                passed &= submit_and_wait(black[update_index], framebuffer, x, y,
                                          size, repetition, "darken");
                passed &= submit_and_wait(paper[update_index], framebuffer, x, y,
                                          size, repetition, "lighten");
            }
        }
    }

    std::memcpy(framebuffer.pixels, saved, byte_count);
    std::free(saved);
    passed &= paper_pro_epaper_submit_update(
                  0, 0, framebuffer.width, framebuffer.height, 1, 4, 1) == 1;
    passed &= paper_pro_epaper_wait_until_update_queue_empty() == 1;
    return passed ? 0 : 6;
}
