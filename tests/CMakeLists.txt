if(WIN32)
  set(CMAKE_RUNTIME_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/bin)
endif()

function(diffy_register_test target_name)
  add_test(NAME ${target_name} COMMAND ${target_name})

  if(WIN32 AND QT6_IS_SHARED_LIBS_BUILD)
    add_custom_command(
      TARGET ${target_name}
      POST_BUILD
      COMMAND ${CMAKE_COMMAND} -E copy_if_different
              $<TARGET_RUNTIME_DLLS:${target_name}>
              $<TARGET_FILE_DIR:${target_name}>
      COMMAND_EXPAND_LISTS
    )
  endif()
endfunction()

qt_add_executable(test_compare_spec
  test_compare_spec.cpp
)

target_include_directories(test_compare_spec PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_compare_spec PRIVATE diffy_core Qt6::Core Qt6::Test)
diffy_register_test(test_compare_spec)

qt_add_executable(test_unified_diff_parser
  test_unified_diff_parser.cpp
)

target_include_directories(test_unified_diff_parser PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_unified_diff_parser PRIVATE diffy_core Qt6::Core Qt6::Test)
diffy_register_test(test_unified_diff_parser)

qt_add_executable(test_text_buffer
  test_text_buffer.cpp
)

target_include_directories(test_text_buffer PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_text_buffer PRIVATE diffy_core Qt6::Core Qt6::Test)
diffy_register_test(test_text_buffer)

qt_add_executable(test_diff_layout_engine
  test_diff_layout_engine.cpp
)

target_include_directories(test_diff_layout_engine PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_diff_layout_engine PRIVATE diffy_core Qt6::Core Qt6::Test)
diffy_register_test(test_diff_layout_engine)

qt_add_executable(test_diff_controller
  test_diff_controller.cpp
)

target_include_directories(test_diff_controller PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_diff_controller PRIVATE
  diffy_app
  Qt6::Test
)
diffy_register_test(test_diff_controller)

qt_add_executable(test_repository_picker_model
  test_repository_picker_model.cpp
)

target_include_directories(test_repository_picker_model PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_repository_picker_model PRIVATE
  diffy_app
  Qt6::Test
)

diffy_register_test(test_repository_picker_model)

qt_add_executable(test_git_diff_backend
  test_git_diff_backend.cpp
)

target_include_directories(test_git_diff_backend PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_git_diff_backend PRIVATE
  diffy_core
  Qt6::Core
  Qt6::Test
)

diffy_register_test(test_git_diff_backend)

qt_add_executable(test_word_diff
  test_word_diff.cpp
)

target_include_directories(test_word_diff PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_word_diff PRIVATE diffy_core Qt6::Core Qt6::Test)
diffy_register_test(test_word_diff)

qt_add_executable(test_highlighter
  test_highlighter.cpp
)

target_include_directories(test_highlighter PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_highlighter PRIVATE diffy_core Qt6::Core Qt6::Test)
diffy_register_test(test_highlighter)

qt_add_executable(test_fuzzy_match
  test_fuzzy_match.cpp
)

target_include_directories(test_fuzzy_match PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../src
)

target_link_libraries(test_fuzzy_match PRIVATE diffy_core Qt6::Core Qt6::Test)
diffy_register_test(test_fuzzy_match)

qt_add_executable(test_app_smoke
  test_app_smoke.cpp
  test_app_smoke.h
)

target_link_libraries(test_app_smoke PRIVATE Qt6::Core Qt6::Gui Qt6::Test)
diffy_register_test(test_app_smoke)
