set(GRAMMAR_SOURCES "")
set(GRAMMAR_INCLUDE_DIRS "")
set(MISSING_GRAMMARS "")

set(GRAMMAR_LANGS
  bash c cpp go javascript json nix python rust toml zig
)

foreach(lang ${GRAMMAR_LANGS})
  set(lang_dir "${CMAKE_CURRENT_SOURCE_DIR}/${lang}")
  set(lang_src_dir "${lang_dir}/src")

  if(NOT EXISTS "${lang_src_dir}/parser.c")
    list(APPEND MISSING_GRAMMARS "${lang}")
    continue()
  endif()

  list(APPEND GRAMMAR_SOURCES "${lang_src_dir}/parser.c")

  if(EXISTS "${lang_src_dir}/scanner.c")
    list(APPEND GRAMMAR_SOURCES "${lang_src_dir}/scanner.c")
  elseif(EXISTS "${lang_src_dir}/scanner.cc")
    list(APPEND GRAMMAR_SOURCES "${lang_src_dir}/scanner.cc")
  endif()

  list(APPEND GRAMMAR_INCLUDE_DIRS "${lang_src_dir}")
endforeach()

if(MISSING_GRAMMARS)
  string(JOIN ", " MISSING_GRAMMARS_TEXT ${MISSING_GRAMMARS})
  message(FATAL_ERROR
    "Missing vendored tree-sitter grammar sources for: ${MISSING_GRAMMARS_TEXT}\n"
    "Configure from the repo root so top-level CMake can materialize them, or run ./scripts/vendor-grammars.sh manually."
  )
endif()

add_library(diffy_grammars STATIC ${GRAMMAR_SOURCES})

set_target_properties(diffy_grammars PROPERTIES
  AUTOMOC OFF
  AUTORCC OFF
  AUTOUIC OFF
  LINKER_LANGUAGE C
)

target_include_directories(diffy_grammars PRIVATE
  ${GRAMMAR_INCLUDE_DIRS}
  ${TREESITTER_INCLUDE_DIRS}
)

target_compile_options(diffy_grammars PRIVATE
  ${TREESITTER_CFLAGS_OTHER}
  -Wno-unused-but-set-variable
  -Wno-unused-parameter
)
