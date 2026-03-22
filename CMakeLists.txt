cmake_minimum_required(VERSION 3.22)
project(diffy LANGUAGES C CXX)

include(GNUInstallDirs)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)
set(CMAKE_AUTOUIC ON)

enable_testing()

option(DIFFY_ENABLE_QML_DEBUG "Enable QML debugging for live preview workflows" OFF)
option(DIFFY_FAST_QML_DEV "Disable QML cache generation for faster local iteration" OFF)
option(DIFFY_AUTO_VENDOR_GRAMMARS "Automatically materialize pinned tree-sitter grammar sources during configure" ON)
option(DIFFY_REGENERATE_GHOSTTY_THEMES
  "Regenerate Ghostty theme dataset during build even when the checked-in dataset exists"
  OFF
)

find_package(Git REQUIRED)
find_package(Qt6 REQUIRED COMPONENTS Core Gui Qml Quick Test Widgets Concurrent)

include(FetchContent)
FetchContent_Declare(
  spdlog
  GIT_REPOSITORY https://github.com/gabime/spdlog.git
  GIT_TAG        v1.15.3
  GIT_SHALLOW    TRUE
)
FetchContent_Declare(
  simdjson
  GIT_REPOSITORY https://github.com/simdjson/simdjson.git
  GIT_TAG        v4.3.1
  GIT_SHALLOW    TRUE
)
set(SPDLOG_FMT_EXTERNAL OFF CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(spdlog simdjson)

if(WIN32)
  find_package(CURL REQUIRED)
  find_package(libgit2 CONFIG REQUIRED)
  find_package(unofficial-tree-sitter CONFIG REQUIRED)
  set(DIFFY_LIBCURL_TARGET CURL::libcurl)
  set(DIFFY_LIBGIT2_TARGET libgit2::libgit2package)
  set(DIFFY_TREESITTER_TARGET unofficial::tree-sitter::tree-sitter)
  set(DIFFY_EXTRA_INCLUDE_DIRS "")
else()
  find_package(PkgConfig REQUIRED)
  pkg_check_modules(LIBCURL REQUIRED IMPORTED_TARGET libcurl)
  pkg_check_modules(LIBGIT2 REQUIRED IMPORTED_TARGET libgit2)
  pkg_check_modules(TREESITTER REQUIRED IMPORTED_TARGET tree-sitter)
  set(DIFFY_LIBCURL_TARGET PkgConfig::LIBCURL)
  set(DIFFY_LIBGIT2_TARGET PkgConfig::LIBGIT2)
  set(DIFFY_TREESITTER_TARGET PkgConfig::TREESITTER)
  set(DIFFY_EXTRA_INCLUDE_DIRS "")
endif()

add_library(diffy_curl INTERFACE)
target_link_libraries(diffy_curl INTERFACE ${DIFFY_LIBCURL_TARGET})

add_library(diffy_libgit2 INTERFACE)
target_link_libraries(diffy_libgit2 INTERFACE ${DIFFY_LIBGIT2_TARGET})

add_library(diffy_tree_sitter INTERFACE)
target_link_libraries(diffy_tree_sitter INTERFACE ${DIFFY_TREESITTER_TARGET})

if(DIFFY_FAST_QML_DEV)
  set(QT_QML_NO_CACHEGEN TRUE)
endif()

set(DIFFY_GRAMMAR_LANGS
  bash c cpp go javascript json nix python rust toml zig
)

set(DIFFY_MISSING_GRAMMARS "")
foreach(lang ${DIFFY_GRAMMAR_LANGS})
  if(NOT EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/vendor/grammars/${lang}/src/parser.c")
    list(APPEND DIFFY_MISSING_GRAMMARS "${lang}")
  endif()
endforeach()

if(DIFFY_MISSING_GRAMMARS)
  if(NOT DIFFY_AUTO_VENDOR_GRAMMARS)
    string(JOIN ", " DIFFY_MISSING_GRAMMARS_TEXT ${DIFFY_MISSING_GRAMMARS})
    message(FATAL_ERROR
      "Missing vendored tree-sitter grammar sources for: ${DIFFY_MISSING_GRAMMARS_TEXT}\n"
      "Run ./scripts/vendor-grammars.sh before configuring the build."
    )
  endif()

  string(JOIN ", " DIFFY_MISSING_GRAMMARS_TEXT ${DIFFY_MISSING_GRAMMARS})
  message(STATUS "Materializing vendored tree-sitter grammars for: ${DIFFY_MISSING_GRAMMARS_TEXT}")
  execute_process(
    COMMAND
      ${CMAKE_COMMAND}
      -DROOT_DIR=${CMAKE_CURRENT_SOURCE_DIR}
      -DLOCK_FILE=${CMAKE_CURRENT_SOURCE_DIR}/vendor/grammars.lock
      -DVENDOR_DIR=${CMAKE_CURRENT_SOURCE_DIR}/vendor/grammars
      -DGIT_EXECUTABLE=${GIT_EXECUTABLE}
      -P ${CMAKE_CURRENT_SOURCE_DIR}/cmake/VendorGrammars.cmake
    WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}"
    COMMAND_ERROR_IS_FATAL ANY
  )
endif()

qt_standard_project_setup()

add_subdirectory(vendor/grammars)

set(DIFFY_SOURCE_THEME_JSON_PATH "${CMAKE_CURRENT_SOURCE_DIR}/src/app/ghostty_themes.json")
set(DIFFY_THEME_JSON_PATH "${DIFFY_SOURCE_THEME_JSON_PATH}")
set(DIFFY_GENERATE_GHOSTTY_THEMES OFF)

if(DIFFY_REGENERATE_GHOSTTY_THEMES)
  set(DIFFY_GENERATE_GHOSTTY_THEMES ON)
elseif(NOT EXISTS "${DIFFY_SOURCE_THEME_JSON_PATH}")
  set(DIFFY_GENERATE_GHOSTTY_THEMES ON)
  message(STATUS
    "Missing checked-in Ghostty theme dataset at ${DIFFY_SOURCE_THEME_JSON_PATH}; "
    "generating it into the build directory."
  )
endif()

if(DIFFY_GENERATE_GHOSTTY_THEMES)
  find_program(DIFFY_PYTHON3_EXECUTABLE NAMES python3 python)
  if(NOT DIFFY_PYTHON3_EXECUTABLE)
    message(FATAL_ERROR
      "Generating the Ghostty theme dataset requires Python 3.\n"
      "Install python3 or restore ${DIFFY_SOURCE_THEME_JSON_PATH}."
    )
  endif()
  set(DIFFY_THEME_JSON_PATH "${CMAKE_CURRENT_BINARY_DIR}/generated/ghostty_themes.json")
  add_custom_command(
    OUTPUT "${DIFFY_THEME_JSON_PATH}"
    COMMAND ${CMAKE_COMMAND} -E make_directory "${CMAKE_CURRENT_BINARY_DIR}/generated"
    COMMAND ${DIFFY_PYTHON3_EXECUTABLE}
            "${CMAKE_CURRENT_SOURCE_DIR}/scripts/generate_ghostty_themes.py"
            --output "${DIFFY_THEME_JSON_PATH}"
    DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/scripts/generate_ghostty_themes.py"
    VERBATIM
  )
  add_custom_target(diffy_ghostty_themes_data DEPENDS "${DIFFY_THEME_JSON_PATH}")
endif()

configure_file(
  "${CMAKE_CURRENT_SOURCE_DIR}/src/app/ThemeProviderThemes.qrc.in"
  "${CMAKE_CURRENT_BINARY_DIR}/generated/ThemeProviderThemes.qrc"
  @ONLY
)

add_library(diffy_core STATIC
  src/core/compare/CompareService.cpp
  src/core/compare/backends/GitDiffBackend.cpp
  src/core/compare/backends/DifftasticBackend.cpp
  src/core/vcs/CompareSpec.cpp
  src/core/diff/DiffTypes.cpp
  src/core/rendering/FlatDiffRows.cpp
  src/core/rendering/PreparedRows.cpp
  src/core/rendering/DiffLayoutEngine.cpp
  src/core/search/FuzzyMatch.cpp
  src/core/vcs/github/GitHubApi.cpp
  src/core/vcs/github/GitHubDeviceFlow.cpp
  src/core/vcs/github/GitHubPullRequest.cpp
  src/core/vcs/git/GitRepositoryService.cpp
  src/core/diff/UnifiedDiffParser.cpp
  src/core/diff/WordDiff.cpp
  src/core/syntax/DiffSyntaxAnnotator.cpp
  src/core/syntax/LanguageRegistry.cpp
  src/core/syntax/Highlighter.cpp
  src/core/text/TextBuffer.cpp
)

target_include_directories(diffy_core PUBLIC
  ${CMAKE_CURRENT_SOURCE_DIR}/src
  ${DIFFY_EXTRA_INCLUDE_DIRS}
)

target_link_libraries(diffy_core PUBLIC
  Qt6::Core
  Qt6::Gui
  diffy_curl
  diffy_libgit2
  diffy_tree_sitter
  diffy_grammars
  spdlog::spdlog
  simdjson::simdjson
)

add_library(diffy_app STATIC
  src/app/DiffController.cpp
  src/app/ThemeProvider.cpp
  "${CMAKE_CURRENT_BINARY_DIR}/generated/ThemeProviderThemes.qrc"
  src/app/QtDiffTypes.cpp
  src/app/models/DiffRowListModel.cpp
  src/app/models/RepositoryPickerModel.cpp
  src/app/quick/DiffSurfaceItem.cpp
)

if(DIFFY_GENERATE_GHOSTTY_THEMES)
  add_dependencies(diffy_app diffy_ghostty_themes_data)
endif()

target_include_directories(diffy_app PUBLIC
  ${CMAKE_CURRENT_SOURCE_DIR}/src
)

target_link_libraries(diffy_app PUBLIC
  diffy_core
  Qt6::Core
  Qt6::Gui
  Qt6::Qml
  Qt6::Quick
  Qt6::Widgets
  Qt6::Concurrent
  diffy_tree_sitter
)

qt_add_executable(diffy
  src/main.cpp
)

set_target_properties(diffy PROPERTIES
  RUNTIME_OUTPUT_DIRECTORY "${CMAKE_BINARY_DIR}/bin"
)

target_link_libraries(diffy PRIVATE diffy_app)

if(DIFFY_ENABLE_QML_DEBUG)
  target_compile_definitions(diffy PRIVATE QT_QML_DEBUG)
endif()

qt_add_qml_module(diffy
  URI Diffy
  VERSION 1.0
  QML_FILES
    qml/Main.qml
    qml/components/ActionButton.qml
    qml/components/Badge.qml
    qml/components/Card.qml
    qml/components/CommandPalette.qml
    qml/components/DebugOverlay.qml
    qml/components/DiffPane.qml
    qml/components/DiffStatsBar.qml
    qml/components/EmptyState.qml
    qml/components/FileListPane.qml
    qml/components/FilterComboBox.qml
    qml/components/Icon.qml
    qml/components/InputField.qml
    qml/components/ProgressBar.qml
    qml/components/RefPickerDropdown.qml
    qml/components/SegmentedControl.qml
    qml/components/ShortcutOverlay.qml
    qml/components/Skeleton.qml
    qml/components/SplitHandle.qml
    qml/components/Toast.qml
    qml/views/WelcomeView.qml
    qml/views/CompareSetupView.qml
    qml/views/DiffBrowsingView.qml
)

if(WIN32 AND QT6_IS_SHARED_LIBS_BUILD)
  install(TARGETS diffy RUNTIME DESTINATION ".")

  qt_generate_deploy_qml_app_script(
    TARGET diffy
    OUTPUT_SCRIPT diffy_deploy_script
    NO_UNSUPPORTED_PLATFORM_ERROR
    DEPLOY_TOOL_OPTIONS --include-plugins qoffscreen,qminimal
  )

  install(CODE "set(QT_DEPLOY_BIN_DIR \".\")")
  install(SCRIPT ${diffy_deploy_script})
  install(CODE "file(MAKE_DIRECTORY \"$ENV{DESTDIR}${CMAKE_INSTALL_PREFIX}/lib/fonts\")")

  add_custom_command(TARGET diffy POST_BUILD
    COMMAND ${CMAKE_COMMAND}
      -D CMAKE_INSTALL_PREFIX=$<TARGET_FILE_DIR:diffy>
      -D QT_DEPLOY_PREFIX=$<TARGET_FILE_DIR:diffy>
      -D QT_DEPLOY_BIN_DIR=.
      -P ${diffy_deploy_script}
    COMMAND ${CMAKE_COMMAND} -E make_directory $<TARGET_FILE_DIR:diffy>/lib/fonts
    VERBATIM
  )
else()
  install(TARGETS diffy RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
endif()

add_subdirectory(tests)
