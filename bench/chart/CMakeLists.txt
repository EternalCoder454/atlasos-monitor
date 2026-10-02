# Chart benchmark (not part of the app build). See bench/chart/README.md.
cmake_minimum_required(VERSION 3.24)
project(chartbench LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_AUTOMOC ON)
find_package(Qt6 6.8 REQUIRED COMPONENTS Core Gui Qml Quick)
qt_policy(SET QTP0001 NEW)
qt_policy(SET QTP0004 NEW)

qt_add_executable(chartbench main.cpp)
qt_add_qml_module(chartbench
    URI ChartBench
    VERSION 1.0
    QML_FILES Main.qml
    SOURCES livechart.h livechart.cpp
)
target_link_libraries(chartbench PRIVATE Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick)
