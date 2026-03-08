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
)

target_include_directories(test_text_rope PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_text_rope PRIVATE diffy_core Qt6::Core Qt6::Test)
add_test(NAME test_text_rope COMMAND test_text_rope)

qt_add_executable(test_diff_layout_engine
  test_diff_layout_engine.cpp
)

target_include_directories(test_diff_layout_engine PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_diff_layout_engine PRIVATE diffy_core Qt6::Core Qt6::Test)
add_test(NAME test_diff_layout_engine COMMAND test_diff_layout_engine)

qt_add_executable(test_diff_controller
  test_diff_controller.cpp
)

target_include_directories(test_diff_controller PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
  ${LIBGIT2_INCLUDE_DIRS}
  ${TREESITTER_INCLUDE_DIRS}
)

target_link_libraries(test_diff_controller PRIVATE
  diffy_app
  Qt6::Test
)
add_test(NAME test_diff_controller COMMAND test_diff_controller)

qt_add_executable(test_repository_picker_model
  test_repository_picker_model.cpp
)

target_include_directories(test_repository_picker_model PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
  ${LIBGIT2_INCLUDE_DIRS}
)

target_link_libraries(test_repository_picker_model PRIVATE
  diffy_app
  Qt6::Test
)

add_test(NAME test_repository_picker_model COMMAND test_repository_picker_model)

qt_add_executable(test_git_diff_backend
  test_git_diff_backend.cpp
)

target_include_directories(test_git_diff_backend PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
  ${LIBGIT2_INCLUDE_DIRS}
)

target_link_libraries(test_git_diff_backend PRIVATE
  diffy_core
  Qt6::Core
  Qt6::Test
)

add_test(NAME test_git_diff_backend COMMAND test_git_diff_backend)

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
