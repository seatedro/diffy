cmake_minimum_required(VERSION 3.22)
project(diffy LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)
set(CMAKE_AUTOUIC ON)

enable_testing()

find_package(PkgConfig REQUIRED)
find_package(Qt6 REQUIRED COMPONENTS Core Gui Qml Quick Test)

pkg_check_modules(LIBGIT2 REQUIRED libgit2)
pkg_check_modules(TREESITTER REQUIRED tree-sitter)

qt_standard_project_setup()

qt_add_executable(diffy
  src/main.cpp
  src/core/CompareSpec.cpp
  src/core/DiffTypes.cpp
  src/core/GitRepositoryService.cpp
  src/core/UnifiedDiffParser.cpp
  src/core/DiffController.cpp
  src/renderers/BuiltinGitRenderer.cpp
  src/renderers/DifftasticRenderer.cpp
)

target_include_directories(diffy PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/src
  ${LIBGIT2_INCLUDE_DIRS}
  ${TREESITTER_INCLUDE_DIRS}
)

target_link_libraries(diffy PRIVATE
  Qt6::Core
  Qt6::Gui
  Qt6::Qml
  Qt6::Quick
  ${LIBGIT2_LIBRARIES}
  ${TREESITTER_LIBRARIES}
)

target_compile_options(diffy PRIVATE ${LIBGIT2_CFLAGS_OTHER} ${TREESITTER_CFLAGS_OTHER})

qt_add_qml_module(diffy
  URI Diffy
  VERSION 1.0
  QML_FILES
    qml/Main.qml
    qml/components/ActionButton.qml
    qml/components/FileListPane.qml
    qml/components/DiffPane.qml
    qml/components/InputField.qml
)

add_subdirectory(tests)
