cmake_minimum_required(VERSION 3.22)
project(diffy LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)
set(CMAKE_AUTOUIC ON)

enable_testing()

find_package(PkgConfig REQUIRED)
find_package(Qt6 REQUIRED COMPONENTS Core Gui Qml Quick Test Widgets)

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

qt_standard_project_setup()

add_subdirectory(vendor/grammars)

add_library(diffy_core STATIC
  src/core/CompareSpec.cpp
  src/core/DiffTypes.cpp
  src/core/GitHubApi.cpp
  src/core/GitHubDeviceFlow.cpp
  src/core/GitHubPullRequest.cpp
  src/core/GitRepositoryService.cpp
  src/core/UnifiedDiffParser.cpp
  src/core/WordDiff.cpp
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
  ${TREESITTER_LIBRARIES}
)

target_compile_options(diffy PRIVATE ${TREESITTER_CFLAGS_OTHER})

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
    qml/components/EmptyState.qml
    qml/components/FileListPane.qml
    qml/components/InputField.qml
    qml/components/PickerOverlay.qml
    qml/components/ProgressBar.qml
    qml/components/RepositoryPickerOverlay.qml
    qml/components/SegmentedControl.qml
    qml/components/ShortcutOverlay.qml
    qml/components/SplitHandle.qml
    qml/components/Toast.qml
    qml/views/WelcomeView.qml
    qml/views/CompareSetupView.qml
    qml/views/DiffBrowsingView.qml
)

add_subdirectory(tests)
