cmake_minimum_required(VERSION 3.24)
project(atlas-monitor VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_AUTOMOC ON)

if(CMAKE_INSTALL_PREFIX_INITIALIZED_TO_DEFAULT)
    set(CMAKE_INSTALL_PREFIX /usr CACHE PATH "" FORCE)
endif()
include(GNUInstallDirs)
find_package(Qt6 6.11 REQUIRED COMPONENTS Core Gui Qml Quick QuickControls2 Widgets)
qt_policy(SET QTP0001 NEW)
qt_policy(SET QTP0004 NEW)
find_package(KF6CoreAddons REQUIRED)
find_package(KF6DBusAddons REQUIRED)
find_package(KF6WindowSystem REQUIRED)
find_package(Corrosion REQUIRED)

set(WORKSPACE_DIR ${CMAKE_CURRENT_SOURCE_DIR}/../..)

# Rust library (CXX-Qt QObjects, atlas-framework, atlas-sysinfo). cxx-qt-build
# finds Qt via $QMAKE.
find_program(QMAKE6 NAMES qmake6 qmake REQUIRED)
corrosion_import_crate(MANIFEST_PATH ${WORKSPACE_DIR}/Cargo.toml
    CRATES atlas-monitor LOCKED)
corrosion_set_env_vars(atlas_monitor QMAKE=${QMAKE6})

# Atlas.Ui, the shared Atlas look, is installed like Kirigami (atlas-ui, from
# atlas-framework): QML imports it and nothing links it. qmlcachegen and
# qmllint read its types from the installed copy.
if(NOT EXISTS "${QT6_INSTALL_PREFIX}/${QT6_INSTALL_QML}/Atlas/Ui/qmldir")
    message(FATAL_ERROR "Atlas.Ui is not installed: install atlas-ui (atlas-framework)")
endif()

qt_add_executable(atlas-monitor-app cpp/main.cpp)
set_target_properties(atlas-monitor-app PROPERTIES OUTPUT_NAME atlas-monitor)

# qmlcachegen compiles every QML file to C++ at build time; the app reads no
# .qml from disk at run time.
set_source_files_properties(qml/Format.qml qml/Hues.qml PROPERTIES QT_QML_SINGLETON_TYPE TRUE)
qt_add_qml_module(atlas-monitor-app
    URI net.eterneon.atlas.monitor
    VERSION 1.0
    QML_FILES
        qml/Main.qml qml/Format.qml qml/OverviewPage.qml qml/CpuPage.qml qml/MemoryPage.qml
        qml/DiskPage.qml qml/NetworkPage.qml qml/GpuPage.qml qml/BatteryPage.qml qml/SensorsPage.qml qml/AppsPage.qml qml/StartupPage.qml qml/ServicesPage.qml qml/DetailsDialog.qml qml/EnergyPage.qml
        qml/SettingsPage.qml qml/AboutPage.qml qml/SystemPage.qml qml/DevicesPage.qml
        qml/Hues.qml qml/ResourcePage.qml
        qml/FigureCard.qml qml/IconButton.qml
    SOURCES
        cpp/coregrid.h cpp/coregrid.cpp cpp/sparkline.h cpp/sparkline.cpp
)
target_include_directories(atlas-monitor-app PRIVATE cpp)

target_link_libraries(atlas-monitor-app PRIVATE
    atlas_monitor
    Qt6::Core Qt6::Gui Qt6::Qml Qt6::Quick Qt6::QuickControls2 Qt6::Widgets
    KF6::CoreAddons KF6::DBusAddons KF6::WindowSystem)

install(TARGETS atlas-monitor-app RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR})
install(FILES data/net.eterneon.atlas.monitor.desktop
        DESTINATION ${CMAKE_INSTALL_DATADIR}/applications)
# Plasma's global shortcuts read X-KDE-Shortcuts from the copies here: the
# desktop file again is how Konsole and Dolphin get theirs.
install(FILES data/net.eterneon.atlas.monitor.desktop
        DESTINATION ${CMAKE_INSTALL_DATADIR}/kglobalaccel)
install(FILES data/net.eterneon.atlas.monitor.metainfo.xml
        DESTINATION ${CMAKE_INSTALL_DATADIR}/metainfo)
install(FILES data/net.eterneon.atlas.monitor.svg
        DESTINATION ${CMAKE_INSTALL_DATADIR}/icons/hicolor/scalable/apps)
# Drawn again for 16 px, where the screen's grid would blur.
install(FILES data/net.eterneon.atlas.monitor-16.svg
        DESTINATION ${CMAKE_INSTALL_DATADIR}/icons/hicolor/16x16/apps
        RENAME net.eterneon.atlas.monitor.svg)
