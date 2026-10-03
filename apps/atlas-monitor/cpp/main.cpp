// Starts Qt, makes the app single-instance, picks the Qt Quick backend and
// loads the window. All app logic is in Rust (src/); this file only glues.
#include <KDBusService>
#include <KWindowSystem>

#include <QApplication>
#include <QEvent>
#include <QIcon>
#include <QPointer>
#include <QQmlApplicationEngine>
#include <QQmlEngine>
#include <QQuickStyle>
#include <QQuickWindow>
#include <QSGRendererInterface>

#include <cstdio>
#include <utility>

// Rust, see src/lib.rs, src/crash.rs, src/logging.rs and src/settings.rs.
struct AtlasObjects {
    void *backend;
    void *sampler;
    void *cpu;
    void *memory;
    void *health;
    void *devices;
    void *disk;
    void *net;
    void *gpu;
    void *battery;
    void *sensors;
    void *apps;
    void *startup;
    void *services;
    void *details;
    void *energy;
};
extern "C" AtlasObjects atlas_objects_new(const char *iconTheme);
extern "C" const char *atlas_icon_search_paths(const char *iconTheme);
extern "C" void atlas_log_init();
extern "C" void atlas_crash_install();
extern "C" void atlas_crash_fatal(const char *msg);
extern "C" bool atlas_settings_gpu_rendering();

// Tells the sampler when the window can't be seen: minimized, hidden, or
// not exposed (KWin suspends a minimized window, or one on another desktop,
// and Qt then takes it as unexposed). Nothing is read meanwhile.
class PauseWhenUnseen : public QObject
{
public:
    PauseWhenUnseen(QQuickWindow *window, QObject *sampler)
        : QObject(window)
        , m_window(window)
        , m_sampler(sampler)
    {
        window->installEventFilter(this);
        connect(window, &QWindow::visibilityChanged, this, &PauseWhenUnseen::update);
    }

protected:
    bool eventFilter(QObject *watched, QEvent *event) override
    {
        // isExposed() already holds the new state when the event arrives.
        if (event->type() == QEvent::Expose) {
            update();
        }
        return QObject::eventFilter(watched, event);
    }

private:
    void update()
    {
        const auto v = m_window->visibility();
        const bool unseen = !m_window->isExposed() || v == QWindow::Minimized || v == QWindow::Hidden;
        if (unseen != m_unseen) {
            m_unseen = unseen;
            QMetaObject::invokeMethod(m_sampler, "setPaused", Q_ARG(bool, unseen));
        }
    }

    QQuickWindow *m_window;
    QObject *m_sampler;
    bool m_unseen = false;
};

static QtMessageHandler s_previousHandler = nullptr;

static void messageHandler(QtMsgType type, const QMessageLogContext &context, const QString &msg)
{
    if (type == QtFatalMsg) {
        // Let the Rust side save a crash report (if the user enabled them).
        atlas_crash_fatal(msg.toUtf8().constData());
    }
    if (s_previousHandler) {
        s_previousHandler(type, context, msg);
    } else {
        // Qt's built-in handler is not returned by qInstallMessageHandler:
        // print the message ourselves, so warnings and fatal errors are not lost.
        fprintf(stderr, "%s\n", qPrintable(qFormatLogMessage(type, context, msg)));
        fflush(stderr);
    }
}

int main(int argc, char *argv[])
{
    // First, before anything can start a thread: Qt's raster engine hands
    // every fill of 96+ spans to a thread pool and waits, and for chart-sized
    // shapes the hand-off costs more than the fill (bench/chart, measured on
    // the software backend).
    if (qEnvironmentVariableIsEmpty("QT_NO_GUI_THREADPOOL")) {
        qputenv("QT_NO_GUI_THREADPOOL", "1");
    }
    atlas_log_init();
    atlas_crash_install(); // Rust panic hook, before anything can panic.
    // Before QApplication: its constructor raises the most common fatal
    // errors (no display, no platform plugin).
    s_previousHandler = qInstallMessageHandler(messageHandler);

    // Draw on the CPU (Qt Quick's software backend) unless the user turned on
    // "Use the graphics card" in Settings: the GPU path loads Mesa and LLVM,
    // tens of MiB for a window of charts. QT_QUICK_BACKEND still overrides.
    if (qEnvironmentVariableIsEmpty("QT_QUICK_BACKEND") && !atlas_settings_gpu_rendering()) {
        QQuickWindow::setGraphicsApi(QSGRendererInterface::Software);
    }

    QApplication app(argc, argv);
    // Together these give the single-instance D-Bus name net.eterneon.atlas.monitor
    // (the app ID); do not change either without changing that name.
    QApplication::setOrganizationDomain(QStringLiteral("atlas.eterneon.net"));
    QApplication::setApplicationName(QStringLiteral("monitor"));
    QApplication::setApplicationDisplayName(QStringLiteral("Atlas Monitor"));
    QApplication::setApplicationVersion(QStringLiteral(ATLAS_MONITOR_VERSION));
    QApplication::setDesktopFileName(QStringLiteral("net.eterneon.atlas.monitor"));
    QApplication::setWindowIcon(QIcon::fromTheme(QStringLiteral("net.eterneon.atlas.monitor")));

    if (qEnvironmentVariableIsEmpty("QT_QUICK_CONTROLS_STYLE")) {
        QQuickStyle::setStyle(QStringLiteral("org.kde.desktop"));
    }

    // One instance per session: a second launch asks this one to show its
    // window (activateRequested) and exits.
    KDBusService service(KDBusService::Unique);

    // Every QObject QML sees, made in Rust; main() owns them, so the QML
    // engine must never delete one. The sampler is listed first: deleting it
    // stops the thread that posts to the others.
    // The Apps table picks its icons the way Qt would, in Qt's theme, and
    // may find one in a folder Qt doesn't search (Flatpak's exports).
    const QByteArray iconTheme = QIcon::themeName().toUtf8();
    {
        QStringList paths = QIcon::themeSearchPaths();
        for (const QString &p : QString::fromUtf8(atlas_icon_search_paths(iconTheme.constData())).split(QLatin1Char('\n'), Qt::SkipEmptyParts)) {
            if (!paths.contains(p)) {
                paths.append(p);
            }
        }
        QIcon::setThemeSearchPaths(paths);
    }
    const AtlasObjects made = atlas_objects_new(iconTheme.constData());
    const std::pair<const char *, void *> objects[] = {
        {"sampler", made.sampler},
        {"backend", made.backend},
        {"cpu", made.cpu},
        {"memory", made.memory},
        {"health", made.health},
        {"devices", made.devices},
        {"disk", made.disk},
        {"net", made.net},
        {"gpu", made.gpu},
        {"battery", made.battery},
        {"sensors", made.sensors},
        {"apps", made.apps},
        {"startup", made.startup},
        {"services", made.services},
        {"details", made.details},
        {"energy", made.energy},
    };
    QVariantMap initial;
    for (const auto &[name, object] : objects) {
        auto *o = static_cast<QObject *>(object);
        QQmlEngine::setObjectOwnership(o, QQmlEngine::CppOwnership);
        initial.insert(QLatin1String(name), QVariant::fromValue(o));
    }

    int rc = 0;
    {
        QQmlApplicationEngine engine;
        engine.setInitialProperties(initial);
        QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed, &app, [] { QCoreApplication::exit(1); }, Qt::QueuedConnection);
        engine.loadFromModule(QStringLiteral("net.eterneon.atlas.monitor"), QStringLiteral("Main"));

        QPointer<QQuickWindow> window = qobject_cast<QQuickWindow *>(engine.rootObjects().value(0));
        if (window) {
            new PauseWhenUnseen(window, static_cast<QObject *>(made.sampler));
            QObject::connect(&service, &KDBusService::activateRequested, window, [window](const QStringList &, const QString &) {
                // KDBusService put the second launch's activation token in the
                // environment; on Wayland, KWin only lets a window take focus
                // with one.
                KWindowSystem::updateStartupId(window);
                window->show();
                window->raise();
                KWindowSystem::activateWindow(window);
            });
            rc = app.exec();
        } else {
            rc = 1;
        }
    }
    for (const auto &[name, object] : objects) {
        delete static_cast<QObject *>(object);
    }
    return rc;
}
