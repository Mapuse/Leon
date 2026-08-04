cmake_minimum_required(VERSION 3.20)
include(CheckIncludeFile)

get_filename_component(REPO_ROOT "${CMAKE_CURRENT_LIST_DIR}" ABSOLUTE)

# ── Auto-detect host architecture ──────────────────────────────────────
execute_process(
  COMMAND uname -m
  OUTPUT_VARIABLE LEON_HOST_ARCH
  OUTPUT_STRIP_TRAILING_WHITESPACE
)

if(LEON_HOST_ARCH STREQUAL "x86_64")
  set(CMAKE_SYSTEM_PROCESSOR x86_64)
  set(LEON_UEFI_TARGET x86_64-unknown-uefi)
elseif(LEON_HOST_ARCH STREQUAL "aarch64")
  set(CMAKE_SYSTEM_PROCESSOR aarch64)
  set(LEON_UEFI_TARGET aarch64-unknown-uefi)
else()
  message(FATAL_ERROR "Unsupported architecture: ${LEON_HOST_ARCH}. Supported: x86_64, aarch64")
endif()

# Leon is built with cargo, not CMake. This file only describes the UEFI
# target triple so any future C component can reuse the arch detection.
set(CMAKE_SYSTEM_NAME Generic)
set(CMAKE_C_COMPILER clang)
set(CMAKE_CXX_COMPILER clang++)
set(CMAKE_C_FLAGS_INIT "-target ${LEON_UEFI_TARGET}")
set(CMAKE_CXX_FLAGS_INIT "-target ${LEON_UEFI_TARGET}")

set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE NEVER)
