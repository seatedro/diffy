qt_add_executable(test_compare_spec
  test_compare_spec.cpp
  ../src/core/CompareSpec.cpp
)

target_include_directories(test_compare_spec PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_compare_spec PRIVATE Qt6::Core Qt6::Test)
add_test(NAME test_compare_spec COMMAND test_compare_spec)

qt_add_executable(test_unified_diff_parser
  test_unified_diff_parser.cpp
  ../src/core/DiffTypes.cpp
  ../src/core/UnifiedDiffParser.cpp
)

target_include_directories(test_unified_diff_parser PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_unified_diff_parser PRIVATE Qt6::Core Qt6::Test)
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

qt_add_executable(test_diff_controller
  test_diff_controller.cpp
  ../src/core/CompareSpec.cpp
  ../src/app/DiffController.cpp
  ../src/core/DiffTypes.cpp
  ../src/core/GitHubPullRequest.cpp
  ../src/core/GitRepositoryService.cpp
  ../src/core/UnifiedDiffParser.cpp
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
  Qt6::Core
  Qt6::Test
  Qt6::Widgets
  ${LIBGIT2_LIBRARIES}
  ${TREESITTER_LIBRARIES}
)

target_compile_options(test_diff_controller PRIVATE ${LIBGIT2_CFLAGS_OTHER} ${TREESITTER_CFLAGS_OTHER})
add_test(NAME test_diff_controller COMMAND test_diff_controller)

qt_add_executable(test_builtin_git_renderer
  test_builtin_git_renderer.cpp
  ../src/core/DiffTypes.cpp
  ../src/renderers/BuiltinGitRenderer.cpp
)

target_include_directories(test_builtin_git_renderer PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
  ${LIBGIT2_INCLUDE_DIRS}
)

target_link_libraries(test_builtin_git_renderer PRIVATE
  Qt6::Core
  Qt6::Test
  ${LIBGIT2_LIBRARIES}
)

target_compile_options(test_builtin_git_renderer PRIVATE ${LIBGIT2_CFLAGS_OTHER})
add_test(NAME test_builtin_git_renderer COMMAND test_builtin_git_renderer)

qt_add_executable(test_app_smoke
  test_app_smoke.cpp
)

target_link_libraries(test_app_smoke PRIVATE Qt6::Core Qt6::Gui Qt6::Test)
add_test(NAME test_app_smoke COMMAND test_app_smoke)
