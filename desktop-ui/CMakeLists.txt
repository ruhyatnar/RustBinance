cmake_minimum_required(VERSION 3.22)
project(binance-bot-ui VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_AUTOMOC ON)
set(CMAKE_AUTORCC ON)

option(BOT_ENABLE_KF6 "Enable KDE Frameworks 6 integration when available" ON)
option(BOT_REQUIRE_KF6 "Fail CMake configuration if KF6 packages are missing" OFF)

find_package(Qt6 REQUIRED COMPONENTS
    Core
    Gui
    Widgets
    Qml
    Quick
    QuickControls2
    Network
    Test
)

set(BOT_HAVE_KF6 0)
if(BOT_ENABLE_KF6 OR BOT_REQUIRE_KF6)
    if(BOT_REQUIRE_KF6)
        find_package(KF6 REQUIRED COMPONENTS StatusNotifierItem Notifications I18n)
    else()
        find_package(KF6 QUIET COMPONENTS StatusNotifierItem Notifications I18n)
    endif()
    if(KF6_FOUND)
        set(BOT_HAVE_KF6 1)
        message(STATUS "KF6 integration enabled (HAVE_KF6=1)")
    else()
        message(STATUS "KF6 not found; using pure Qt6 fallback (HAVE_KF6=0)")
    endif()
endif()

add_library(binance-bot-ui-core STATIC
    src/DecimalFormat.h
    src/DecimalFormat.cpp
    src/Models.h
    src/Models.cpp
    src/StateStore.h
    src/StateStore.cpp
)

target_include_directories(binance-bot-ui-core PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/src")
target_compile_definitions(binance-bot-ui-core PUBLIC HAVE_KF6=${BOT_HAVE_KF6})
target_compile_options(binance-bot-ui-core PRIVATE -Wall -Wextra -Wpedantic)
target_link_libraries(binance-bot-ui-core PUBLIC
    Qt6::Core
    Qt6::Gui
    Qt6::Widgets
    Qt6::Network
)
if(BOT_HAVE_KF6)
    target_link_libraries(binance-bot-ui-core PUBLIC
        KF6::StatusNotifierItem
        KF6::Notifications
        KF6::I18n
    )
endif()

enable_testing()

get_filename_component(BOT_REPO_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/.." ABSOLUTE)

add_executable(tst_decimal_and_store
    tests/tst_decimal_and_store.cpp
)
target_compile_definitions(tst_decimal_and_store PRIVATE
    BOT_REPO_ROOT="${BOT_REPO_ROOT}"
)
target_compile_options(tst_decimal_and_store PRIVATE -Wall -Wextra -Wpedantic)
target_link_libraries(tst_decimal_and_store PRIVATE
    binance-bot-ui-core
    Qt6::Test
)
add_test(NAME tst_decimal_and_store COMMAND tst_decimal_and_store)
