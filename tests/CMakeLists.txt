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
