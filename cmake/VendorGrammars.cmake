cmake_minimum_required(VERSION 3.22)

foreach(required_var ROOT_DIR LOCK_FILE VENDOR_DIR GIT_EXECUTABLE)
  if(NOT DEFINED ${required_var} OR "${${required_var}}" STREQUAL "")
    message(FATAL_ERROR "VendorGrammars.cmake requires ${required_var} to be set")
  endif()
endforeach()

if(NOT EXISTS "${LOCK_FILE}")
  message(FATAL_ERROR "Missing lock file: ${LOCK_FILE}")
endif()

file(MAKE_DIRECTORY "${VENDOR_DIR}")

set(TEMP_DIR "${ROOT_DIR}/build/vendor-grammars-tmp")
file(REMOVE_RECURSE "${TEMP_DIR}")
file(MAKE_DIRECTORY "${TEMP_DIR}")

file(READ "${LOCK_FILE}" LOCK_CONTENTS)
string(REPLACE "\r\n" "\n" LOCK_CONTENTS "${LOCK_CONTENTS}")
string(REPLACE "\r" "\n" LOCK_CONTENTS "${LOCK_CONTENTS}")
string(REPLACE "\n" ";" LOCK_LINES "${LOCK_CONTENTS}")

foreach(line IN LISTS LOCK_LINES)
  string(STRIP "${line}" line)
  if(line STREQUAL "" OR line MATCHES "^#")
    continue()
  endif()

  string(REGEX MATCH "^([^ ]+) +([^ ]+) +([^ ]+)$" _ "${line}")
  if(NOT CMAKE_MATCH_COUNT EQUAL 3)
    message(FATAL_ERROR "Malformed grammar lock entry: ${line}")
  endif()

  set(lang "${CMAKE_MATCH_1}")
  set(url "${CMAKE_MATCH_2}")
  set(commit "${CMAKE_MATCH_3}")
  set(dest "${VENDOR_DIR}/${lang}")
  set(clone_dir "${TEMP_DIR}/${lang}")

  message(STATUS "Fetching ${lang} at ${commit} from ${url}")
  execute_process(COMMAND "${GIT_EXECUTABLE}" init -q "${clone_dir}" COMMAND_ERROR_IS_FATAL ANY)
  execute_process(COMMAND "${GIT_EXECUTABLE}" -C "${clone_dir}" remote add origin "${url}" COMMAND_ERROR_IS_FATAL ANY)
  execute_process(COMMAND "${GIT_EXECUTABLE}" -C "${clone_dir}" fetch -q --depth=1 origin "${commit}"
                  COMMAND_ERROR_IS_FATAL ANY)
  execute_process(COMMAND "${GIT_EXECUTABLE}" -C "${clone_dir}" checkout -q FETCH_HEAD COMMAND_ERROR_IS_FATAL ANY)

  file(MAKE_DIRECTORY "${dest}")
  file(REMOVE_RECURSE "${dest}/src")
  file(MAKE_DIRECTORY "${dest}/src" "${dest}/queries")

  if(EXISTS "${clone_dir}/src/parser.c")
    file(COPY "${clone_dir}/src/parser.c" DESTINATION "${dest}/src")
  endif()

  if(EXISTS "${clone_dir}/src/scanner.c")
    file(COPY "${clone_dir}/src/scanner.c" DESTINATION "${dest}/src")
  elseif(EXISTS "${clone_dir}/src/scanner.cc")
    file(COPY "${clone_dir}/src/scanner.cc" DESTINATION "${dest}/src")
  endif()

  if(EXISTS "${clone_dir}/src/tree_sitter")
    file(COPY "${clone_dir}/src/tree_sitter" DESTINATION "${dest}/src")
  endif()

  file(REMOVE "${dest}/queries/highlights.scm")
  if(EXISTS "${clone_dir}/queries/highlights.scm")
    file(COPY "${clone_dir}/queries/highlights.scm" DESTINATION "${dest}/queries")
  endif()

  message(STATUS "  -> ${dest}")
endforeach()

file(REMOVE_RECURSE "${TEMP_DIR}")
message(STATUS "Done. Vendored grammars from ${LOCK_FILE}.")
