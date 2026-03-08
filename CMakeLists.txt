cmake_minimum_required(VERSION 3.22)
project(diffy LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)
set(CMAKE_AUTOUIC ON)

enable_testing()

option(DIFFY_ENABLE_QML_DEBUG "Enable QML debugging for live preview workflows" OFF)
option(DIFFY_FAST_QML_DEV "Disable QML cache generation for faster local iteration" OFF)

find_package(PkgConfig REQUIRED)
find_package(Qt6 REQUIRED COMPONENTS Core Gui Qml Quick Test Widgets Concurrent)

pkg_check_modules(LIBCURL REQUIRED libcurl)
pkg_check_modules(LIBGIT2 REQUIRED libgit2)
pkg_check_modules(TREESITTER REQUIRED tree-sitter)

include(FetchContent)
FetchContent_Declare(
  spdlog
  GIT_REPOSITORY https://github.com/gabime/spdlog.git
  GIT_TAG        v1.15.3
  GIT_SHALLOW    TRUE
)
set(SPDLOG_FMT_EXTERNAL OFF CACHE BOOL "" FORCE)
FetchContent_MakeAvailable(spdlog)

if(DIFFY_FAST_QML_DEV)
  set(QT_QML_NO_CACHEGEN TRUE)
endif()

qt_standard_project_setup()

add_subdirectory(vendor/grammars)

add_library(diffy_core STATIC
  src/core/vcs/CompareSpec.cpp
  src/core/diff/DiffTypes.cpp
  src/core/search/FuzzyMatch.cpp
  src/core/vcs/github/GitHubApi.cpp
  src/core/vcs/github/GitHubDeviceFlow.cpp
  src/core/vcs/github/GitHubPullRequest.cpp
  src/core/vcs/git/GitRepositoryService.cpp
  src/core/diff/UnifiedDiffParser.cpp
  src/core/diff/WordDiff.cpp
  src/core/syntax/LanguageRegistry.cpp
  src/core/syntax/Highlighter.cpp
)

target_include_directories(diffy_core PUBLIC
  ${CMAKE_CURRENT_SOURCE_DIR}/src
  ${LIBCURL_INCLUDE_DIRS}
  ${LIBGIT2_INCLUDE_DIRS}
  ${TREESITTER_INCLUDE_DIRS}
)

target_link_libraries(diffy_core PUBLIC
  ${LIBCURL_LIBRARIES}
  ${LIBGIT2_LIBRARIES}
  ${TREESITTER_LIBRARIES}
  diffy_grammars
  spdlog::spdlog
)

target_compile_options(diffy_core PRIVATE ${LIBCURL_CFLAGS_OTHER} ${LIBGIT2_CFLAGS_OTHER} ${TREESITTER_CFLAGS_OTHER})

qt_add_executable(diffy
  src/main.cpp
  src/app/DiffController.cpp
  src/app/ThemeProvider.cpp
  src/app/RepositoryPickerModel.cpp
  src/app/QtDiffTypes.cpp
  src/model/DiffPreparedRows.cpp
  src/model/DiffDisplayModel.cpp
  src/model/DiffRowListModel.cpp
  src/text/TextRope.cpp
  src/ui/DiffSurfaceItem.cpp
  src/renderers/BuiltinGitRenderer.cpp
  src/renderers/DifftasticRenderer.cpp
)

target_include_directories(diffy PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/src
  ${TREESITTER_INCLUDE_DIRS}
)

target_link_libraries(diffy PRIVATE
  diffy_core
  Qt6::Core
  Qt6::Gui
  Qt6::Qml
  Qt6::Quick
  Qt6::Widgets
  Qt6::Concurrent
  ${TREESITTER_LIBRARIES}
)

target_compile_options(diffy PRIVATE ${TREESITTER_CFLAGS_OTHER})

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
    qml/components/DiffPane.qml
    qml/components/DiffStatsBar.qml
    qml/components/EmptyState.qml
    qml/components/FileListPane.qml
    qml/components/Icon.qml
    qml/components/InputField.qml
    qml/components/ProgressBar.qml
    qml/components/SegmentedControl.qml
    qml/components/ShortcutOverlay.qml
    qml/components/Skeleton.qml
    qml/components/SplitHandle.qml
    qml/components/Toast.qml
    qml/views/WelcomeView.qml
    qml/views/CompareSetupView.qml
    qml/views/DiffBrowsingView.qml
)

add_subdirectory(tests)
