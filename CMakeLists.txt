cmake_minimum_required(VERSION 3.16...3.27)

set(CMAKE_COMPILE_WARNING_AS_ERROR ON)

set(TARGET_NAME new_project_name)
project(${TARGET_NAME} LANGUAGES CXX)

set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

if(NOT DEFINED CMAKE_CXX_STANDARD)
    set(CMAKE_CXX_STANDARD 17)
endif()

# Rerun:
include(FetchContent)
FetchContent_Declare(rerun_sdk URL https://github.com/rerun-io/rerun/releases/download/0.23.0/rerun_cpp_sdk.zip)
FetchContent_MakeAvailable(rerun_sdk)

add_executable(${TARGET_NAME} src/main.cpp)
target_link_libraries(${TARGET_NAME} rerun_sdk)
target_include_directories(${TARGET_NAME} PRIVATE src)



if(MSVC)
    # TODO(andreas): Try to enable /Wall
    target_compile_options(${TARGET_NAME} PRIVATE /W4)

    target_compile_options(${TARGET_NAME} PRIVATE /we4996) # Using deprecated functions is an error

    if(BUILD_SHARED_LIBS)
        # If we are building as shared libs, we are going to have to disable the C4251
        # warning, as it would trigger for any datatype derived from a STL class
        # See also https://github.com/protocolbuffers/protobuf/blob/v26.1/cmake/README.md#notes-on-compiler-warnings
        # We need also to make it public, otherwise downstream code will be flooded by c4251 warnings
        target_compile_options(${TARGET_NAME} PUBLIC /wd4251)
    endif()

    # CMAKE_COMPILE_WARNING_AS_ERROR is only directly supported starting in CMake `3.24`
    # https://cmake.org/cmake/help/latest/prop_tgt/COMPILE_WARNING_AS_ERROR.html
    if(CMAKE_COMPILE_WARNING_AS_ERROR)
        target_compile_options(${TARGET_NAME} PRIVATE /WX
            /w15038 # Initialization order. https://learn.microsoft.com/en-us/cpp/error-messages/compiler-warnings/c5038
        )
    endif()
else()
    # Enabled warnings.
    target_compile_options(${TARGET_NAME} PRIVATE
        -Wall
        -Wcast-align
        -Wcast-qual
        -Wdeprecated
        -Wdeprecated-declarations
        -Wextra
        -Wformat=2
        -Wmissing-include-dirs
        -Wnull-dereference
        -Wold-style-cast
        -Wpedantic
        -Wpointer-arith
        -Wshadow
        -Wswitch-enum
        -Wunreachable-code
        -Wvla
    )

    if(CMAKE_CXX_COMPILER_ID MATCHES "Clang") # match both "Clang" and "AppleClang"
        # TODO(emilk): enable some hardening flags from https://best.openssf.org/Compiler-Hardening-Guides/Compiler-Options-Hardening-Guide-for-C-and-C++.html
        target_compile_options(${TARGET_NAME} PRIVATE
            -Wc++17-compat-pedantic
            -Wc++20-compat-pedantic
            -Wc99-extensions
            -Weverything
            -Wgnu
            -Wnon-gcc
            -Wpre-c2x-compat-pedantic
            -Wshadow-all

            # Turn off some warning that -Weverything turns on:
            -Wno-c++98-compat
            -Wno-c++98-compat-pedantic
            -Wno-covered-switch-default # We often add a `default:` case out of paranoia
            -Wno-ctad-maybe-unsupported
            -Wno-disabled-macro-expansion
            -Wno-documentation
            -Wno-documentation-unknown-command
            -Wno-double-promotion # float->double is nothing to whine about
            -Wno-exit-time-destructors
            -Wno-float-equal # comparing floats is fine
            -Wno-global-constructors
            -Wno-missing-prototypes
            -Wno-padded
            -Wno-reserved-id-macro
            -Wno-reserved-identifier
            -Wno-unused-macros
            -Wno-unsafe-buffer-usage # Not sure why we need this, but we do.
            -Wno-unknown-warning-option # Otherwise older clang will complain about `-Wno-unsafe-buffer-usage`
        )
    endif()

    # CMAKE_COMPILE_WARNING_AS_ERROR is only directly supported starting in CMake `3.24`
    # https://cmake.org/cmake/help/latest/prop_tgt/COMPILE_WARNING_AS_ERROR.html
    if(CMAKE_COMPILE_WARNING_AS_ERROR)
        target_compile_options(${TARGET_NAME} PRIVATE -Werror)
    endif()

    if(CMAKE_BUILD_TYPE STREQUAL "Debug")
        # Improve stack traces:
        target_compile_options(${TARGET_NAME} PRIVATE -g -fno-omit-frame-pointer -mno-omit-leaf-frame-pointer -fno-optimize-sibling-calls)
    endif()
endif()
