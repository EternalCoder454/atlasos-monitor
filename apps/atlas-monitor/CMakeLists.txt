cmake_minimum_required(VERSION 3.24)
project(atlas-monitor VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

if(CMAKE_INSTALL_PREFIX_INITIALIZED_TO_DEFAULT)
    set(CMAKE_INSTALL_PREFIX /usr CACHE PATH "" FORCE)
endif()
include(GNUInstallDirs)
include(FetchContent)
find_package(Qt6 6.8 REQUIRED COMPONENTS Core Gui Qml Quick QuickControls2 Widgets)
qt_policy(SET QTP0001 NEW)
qt_policy(SET QTP0004 NEW)
find_package(KF6DBusAddons REQUIRED)
find_package(KF6WindowSystem REQUIRED)
find_package(Corrosion REQUIRED)

set(WORKSPACE_DIR ${CMAKE_CURRENT_SOURCE_DIR}/../..)
# Both QML modules (Atlas.Ui and the app's) build under one import path, so
# qmllint and qmlcachegen resolve Atlas.Ui's types when compiling the pages.
set(QT_QML_OUTPUT_DIRECTORY ${CMAKE_BINARY_DIR}/qml)

# Rust library (CXX-Qt QObjects, atlas-core, atlas-sysinfo). cxx-qt-build
# finds Qt via $QMAKE.
find_program(QMAKE6 NAMES qmake6 qmake REQUIRED)
corrosion_import_crate(MANIFEST_PATH ${WORKSPACE_DIR}/Cargo.toml
    CRATES atlas-monitor LOCKED)
corrosion_set_env_vars(atlas_monitor QMAKE=${QMAKE6})

# Atlas.Ui comes from the same atlasos-updater commit as atlas-core. The commit
# is read from the workspace Cargo.toml, so the two can't drift apart. To try
# unpushed Atlas.Ui changes, point CMake at a local checkout:
#   -DFETCHCONTENT_SOURCE_DIR_ATLASOS_UPDATER=/path/to/atlasos-updater
file(READ ${WORKSPACE_DIR}/Cargo.toml _workspace_toml)
string(REGEX MATCH "atlas-core = {[^}]*rev = \"([0-9a-f]+)\"" _pin "${_workspace_toml}")
set(ATLAS_UPDATER_REV "${CMAKE_MATCH_1}")
string(LENGTH "${ATLAS_UPDATER_REV}" _rev_len)
if(NOT _rev_len EQUAL 40)
    message(FATAL_ERROR "No full 40-character atlas-core rev in ${WORKSPACE_DIR}/Cargo.toml")
endif()
message(STATUS "Atlas.Ui from atlasos-updater ${ATLAS_UPDATER_REV}")
set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS ${WORKSPACE_DIR}/Cargo.toml)
FetchContent_Declare(atlasos_updater
    GIT_REPOSITORY https://github.com/EternalCoder454/atlasos-updater.git
    GIT_TAG ${ATLAS_UPDATER_REV}
    SOURCE_SUBDIR ui)
FetchContent_MakeAvailable(atlasos_updater)

qt_add_executable(atlas-monitor-app cpp/main.cpp)
set_target_properties(atlas-monitor-app PROPERTIES OUTPUT_NAME atlas-monitor)
target_compile_definitions(atlas-monitor-app PRIVATE ATLAS_MONITOR_VERSION="${PROJECT_VERSION}")

# qmlcachegen compiles every QML file to C++ at build time; the app reads no
# .qml from disk at run time.
set_source_files_properties(qml/Format.qml PROPERTIES QT_QML_SINGLETON_TYPE TRUE)
qt_add_qml_module(atlas-monitor-app
    URI net.eterneon.atlas.monitor
    VERSION 1.0
    # Atlas.Ui is linked explicitly (atlas-uiplugin); Kirigami and Qt load
    # their plugins at run time. No static plugin import scan needed.
    NO_IMPORT_SCAN
    QML_FILES
        qml/Main.qml qml/Format.qml qml/OverviewPage.qml qml/CpuPage.qml qml/MemoryPage.qml
        qml/DiskPage.qml qml/NetworkPage.qml
        qml/SettingsPage.qml qml/AboutPage.qml
)

target_link_libraries(atlas-monitor-app PRIVATE
    atlas_monitor atlas-ui atlas-uiplugin
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Widgets
    KF6::DBusAddons KF6::WindowSystem)

install(TARGETS atlas-monitor-app RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
install(FILES data/net.eterneon.atlas.monitor.desktop
        DESTINATION ${CMAKE_INSTALL_DATADIR}/applications)
install(FILES data/net.eterneon.atlas.monitor.metainfo.xml
        DESTINATION ${CMAKE_INSTALL_DATADIR}/metainfo)
install(FILES data/net.eterneon.atlas.monitor.svg
        DESTINATION ${CMAKE_INSTALL_DATADIR}/icons/hicolor/scalable/apps)
# Drawn again for 16 px, where the screen's grid would blur.
install(FILES data/net.eterneon.atlas.monitor-16.svg
        DESTINATION ${CMAKE_INSTALL_DATADIR}/icons/hicolor/16x16/apps
        RENAME net.eterneon.atlas.monitor.svg)
