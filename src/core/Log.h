#pragma once

#include <memory>
#include <string>

#include <spdlog/spdlog.h>
#include <spdlog/sinks/rotating_file_sink.h>
#include <spdlog/sinks/stdout_color_sinks.h>

namespace diffy::log {

inline void init(const std::string& logPath) {
  auto fileSink = std::make_shared<spdlog::sinks::rotating_file_sink_mt>(logPath, 5 * 1024 * 1024, 3);
  auto consoleSink = std::make_shared<spdlog::sinks::stderr_color_sink_mt>();
  consoleSink->set_level(spdlog::level::warn);

  auto logger = std::make_shared<spdlog::logger>("diffy",
      spdlog::sinks_init_list{fileSink, consoleSink});
  logger->set_level(spdlog::level::debug);
  logger->set_pattern("[%H:%M:%S.%e] [%^%l%$] %v");
  logger->flush_on(spdlog::level::warn);

  spdlog::set_default_logger(logger);
}

inline void setLevel(spdlog::level::level_enum level) {
  spdlog::set_level(level);
}

template <typename... Args>
void debug(std::string_view category, spdlog::format_string_t<Args...> fmt, Args&&... args) {
  spdlog::debug("[{}] {}", category, fmt::format(fmt, std::forward<Args>(args)...));
}

template <typename... Args>
void info(std::string_view category, spdlog::format_string_t<Args...> fmt, Args&&... args) {
  spdlog::info("[{}] {}", category, fmt::format(fmt, std::forward<Args>(args)...));
}

template <typename... Args>
void warn(std::string_view category, spdlog::format_string_t<Args...> fmt, Args&&... args) {
  spdlog::warn("[{}] {}", category, fmt::format(fmt, std::forward<Args>(args)...));
}

template <typename... Args>
void error(std::string_view category, spdlog::format_string_t<Args...> fmt, Args&&... args) {
  spdlog::error("[{}] {}", category, fmt::format(fmt, std::forward<Args>(args)...));
}

}  // namespace diffy::log
