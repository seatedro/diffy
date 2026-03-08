qt_add_executable(test_compare_spec
  test_compare_spec.cpp
)

target_include_directories(test_compare_spec PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_compare_spec PRIVATE diffy_core Qt6::Core Qt6::Test)
add_test(NAME test_compare_spec COMMAND test_compare_spec)

qt_add_executable(test_unified_diff_parser
  test_unified_diff_parser.cpp
)

target_include_directories(test_unified_diff_parser PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_unified_diff_parser PRIVATE diffy_core Qt6::Core Qt6::Test)
add_test(NAME test_unified_diff_parser COMMAND test_unified_diff_parser)

qt_add_executable(test_text_rope
  test_text_rope.cpp
  ../src/text/TextRope.cpp
)

target_include_directories(test_text_rope PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_text_rope PRIVATE Qt6::Core Qt6::Test)
add_test(NAME test_text_rope COMMAND test_text_rope)

qt_add_executable(test_diff_display_model
  test_diff_display_model.cpp
  ../src/model/DiffDisplayModel.cpp
)

target_include_directories(test_diff_display_model PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_diff_display_model PRIVATE Qt6::Core Qt6::Test)
add_test(NAME test_diff_display_model COMMAND test_diff_display_model)

qt_add_executable(test_diff_controller
  test_diff_controller.cpp
  ../src/app/DiffController.cpp
  ../src/app/RepositoryPickerModel.cpp
  ../src/app/QtDiffTypes.cpp
  ../src/model/DiffRowListModel.cpp
  ../src/renderers/BuiltinGitRenderer.cpp
  ../src/renderers/DifftasticRenderer.cpp
)

target_include_directories(test_diff_controller PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
  ${LIBGIT2_INCLUDE_DIRS}
  ${TREESITTER_INCLUDE_DIRS}
)

target_link_libraries(test_diff_controller PRIVATE
  diffy_core
  Qt6::Core
  Qt6::Test
  Qt6::Widgets
  ${TREESITTER_LIBRARIES}
)

target_compile_options(test_diff_controller PRIVATE ${TREESITTER_CFLAGS_OTHER})
add_test(NAME test_diff_controller COMMAND test_diff_controller)

qt_add_executable(test_repository_picker_model
  test_repository_picker_model.cpp
  ../src/app/RepositoryPickerModel.cpp
)

target_include_directories(test_repository_picker_model PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
  ${LIBGIT2_INCLUDE_DIRS}
)

target_link_libraries(test_repository_picker_model PRIVATE
  Qt6::Core
  Qt6::Test
  ${LIBGIT2_LIBRARIES}
)

target_compile_options(test_repository_picker_model PRIVATE ${LIBGIT2_CFLAGS_OTHER})
add_test(NAME test_repository_picker_model COMMAND test_repository_picker_model)

qt_add_executable(test_builtin_git_renderer
  test_builtin_git_renderer.cpp
  ../src/renderers/BuiltinGitRenderer.cpp
)

target_include_directories(test_builtin_git_renderer PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
  ${LIBGIT2_INCLUDE_DIRS}
)

target_link_libraries(test_builtin_git_renderer PRIVATE
  diffy_core
  Qt6::Core
  Qt6::Test
)

add_test(NAME test_builtin_git_renderer COMMAND test_builtin_git_renderer)

qt_add_executable(test_word_diff
  test_word_diff.cpp
)

target_include_directories(test_word_diff PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_word_diff PRIVATE diffy_core Qt6::Core Qt6::Test)
add_test(NAME test_word_diff COMMAND test_word_diff)

qt_add_executable(test_highlighter
  test_highlighter.cpp
)

target_include_directories(test_highlighter PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
  ${TREESITTER_INCLUDE_DIRS}
)

target_link_libraries(test_highlighter PRIVATE diffy_core Qt6::Core Qt6::Test)
add_test(NAME test_highlighter COMMAND test_highlighter)

qt_add_executable(test_fuzzy_match
  test_fuzzy_match.cpp
)

target_include_directories(test_fuzzy_match PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_fuzzy_match PRIVATE diffy_core Qt6::Core Qt6::Test)
add_test(NAME test_fuzzy_match COMMAND test_fuzzy_match)

qt_add_executable(test_app_smoke
  test_app_smoke.cpp
  test_app_smoke.h
)

target_link_libraries(test_app_smoke PRIVATE Qt6::Core Qt6::Gui Qt6::Test)
add_test(NAME test_app_smoke COMMAND test_app_smoke)
