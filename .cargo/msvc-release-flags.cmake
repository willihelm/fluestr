# Put the optimisation back into MSVC Release builds of whisper.cpp (P18).
#
# whisper-rs-sys builds whisper.cpp through the `cmake` crate, which with the
# Visual Studio generator overwrites CMAKE_<LANG>_FLAGS_<BUILD_TYPE> with cc's
# flags - after stripping every `/O...` from them, on the assumption that CMake
# supplies its own. It does not: the variable it just replaced is where CMake
# keeps `/O2`. A release build of the sidecar therefore compiled ggml without
# any optimisation and decoded a one-second segment in over a minute on CPU.
#
# Loaded into every project() by CMAKE_PROJECT_INCLUDE (.cargo/config.toml),
# after the command line has set the cache, so appending here wins. MSVC only:
# on macOS this file does nothing and Metal builds exactly as before.
if(MSVC)
  foreach(lang C CXX)
    string(APPEND CMAKE_${lang}_FLAGS_RELEASE " /O2 /Ob2 /DNDEBUG")
  endforeach()
endif()
