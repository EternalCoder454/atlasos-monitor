// Starts Qt, makes the app single-instance, picks the Qt Quick backend and
// loads the window. All app logic is in Rust (src/); this file only glues.
#include <KCoreAddons>
#include <KDBusService>
#include <KWindowSystem>

#include <QApplication>
#include <QClipboard>
#include <QCommandLineParser>
#include <QEvent>
#include <QIcon>
#include <QPointer>
#include <QQmlApplicationEngine>
#include <QQmlEngine>
#include <QQuickWindow>
#include <QSGRendererInterface>
#include <QSocketNotifier>

#include <cerrno>
#include <csignal>
#include <fcntl.h>
#include <unistd.h>
#include <utility>

// Rust, see src/lib.rs and src/settings.rs.
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
    void *system;
    void *hardware;
};
extern "C" AtlasObjects atlas_objects_new(const char *iconTheme);
extern "C" const char *atlas_icon_search_paths(const char *iconTheme);
extern "C" bool atlas_settings_gpu_rendering();
extern "C" void atlas_settings_flush();
// telamon-framework-ui (include/telamon/app.h), linked in with the Rust library.
extern "C" void telamon_app_init();
extern "C" void telamon_app_ready();

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

// Asked to quit by a signal (SIGTERM from kill or systemd, SIGINT, SIGHUP):
// close the window as the user would, so its state is saved and Energy Saver
// puts back what it eased. Left to the default, the process just dies, and
// the eases wait for the next start. The handler only writes to a pipe; the
// event loop does the rest. A second signal kills as before (SA_RESETHAND),
// in case quitting itself hangs.
static int s_quitPipe[2] = {-1, -1};

static void onQuitSignal(int)
{
    const int saved = errno;
    const char b = 1;
    [[maybe_unused]] const ssize_t n = ::write(s_quitPipe[1], &b, 1);
    errno = saved;
}

static void closeOnQuitSignals(QQuickWindow *window)
{
    if (::pipe2(s_quitPipe, O_CLOEXEC | O_NONBLOCK) != 0) {
        return;
    }
    auto *notifier = new QSocketNotifier(s_quitPipe[0], QSocketNotifier::Read, window);
    QObject::connect(notifier, &QSocketNotifier::activated, window, [window] {
        char buf[16];
        while (::read(s_quitPipe[0], buf, sizeof buf) > 0) {
        }
        // close() does nothing to a window that isn't shown.
        if (window->isVisible()) {
            window->close();
        } else {
            QCoreApplication::quit();
        }
    });
    struct sigaction sa = {};
    sa.sa_handler = onQuitSignal;
    sigemptyset(&sa.sa_mask);
    sa.sa_flags = SA_RESTART | SA_RESETHAND;
    for (int sig : {SIGTERM, SIGINT, SIGHUP}) {
        sigaction(sig, &sa, nullptr);
    }
}

// Whether the icon theme has an icon, for QML to pick its set of icons by
// (Main.qml, icons).
class ThemeIcons : public QObject
{
    Q_OBJECT
public:
    using QObject::QObject;
    Q_INVOKABLE bool has(const QString &name) const
    {
        return QIcon::hasThemeIcon(name);
    }
};

// What QML can't find out itself, for System Info: the versions of Qt and
// KDE Frameworks this process runs on, the window system, and copying its
// details to the clipboard.
class Platform : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QString qtVersion READ qtVersion CONSTANT)
    Q_PROPERTY(QString frameworksVersion READ frameworksVersion CONSTANT)
    Q_PROPERTY(QString windowSystem READ windowSystem CONSTANT)
public:
    using QObject::QObject;
    QString qtVersion() const
    {
        return QString::fromLatin1(qVersion());
    }
    QString frameworksVersion() const
    {
        return KCoreAddons::versionString();
    }
    QString windowSystem() const
    {
        const QString p = QGuiApplication::platformName();
        if (p.startsWith(QLatin1String("wayland"))) {
            return QStringLiteral("Wayland");
        }
        if (p == QLatin1String("xcb")) {
            return QStringLiteral("X11");
        }
        return p;
    }
    Q_INVOKABLE void copy(const QString &text) const
    {
        QGuiApplication::clipboard()->setText(text);
    }
};

// The page asked for with --page, as Main.qml's showPage takes it ("system",
// "disk:nvme0n1"); "" for none. Main.qml checks it is a page: anything else
// opens Overview. Only the length is bounded here, as no page key is long.
static QString pageOption(const QCommandLineParser &parser, const QCommandLineOption &page)
{
    const QString name = parser.value(page).trimmed();
    return name.size() <= 256 ? name : QString();
}

static QCommandLineOption pageOptionSpec()
{
    return QCommandLineOption(QStringLiteral("page"),
                              QCoreApplication::translate("main", "Open the page <name>: overview, cpu, memory, system, devices, apps, services, …"),
                              QStringLiteral("name"));
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
    // The software renderer repaints only what changed, except at a
    // fractional scale, where it repaints the whole window on every change
    // for fear of seams. At 1.5x that made the sidebar's figures alone cost
    // 11-13 ms/s on every page; partially, 3.5 (bench/pages). Nothing seams
    // inside the window; its edge is seen to in Main.qml (edgeFill).
    if (qEnvironmentVariableIsEmpty("QSG_SOFTWARE_RENDERER_FORCE_PARTIAL_UPDATES")) {
        qputenv("QSG_SOFTWARE_RENDERER_FORCE_PARTIAL_UPDATES", "1");
    }
    // Before QApplication, whose constructor raises the most common fatal
    // errors (no display, no platform plugin): the journal logger, the crash
    // hooks for Rust panics and fatal Qt messages (a report only when the
    // user turned them on, in Atlas Updater), the app ID as organization
    // domain and application name (together the single-instance D-Bus name
    // net.eterneon.atlas.monitor) and desktop file name, the version, and
    // the org.kde.desktop style.
    telamon_app_init();

    // Draw on the CPU (Qt Quick's software backend) unless the user turned on
    // "Use the graphics card" in Settings: the GPU path loads Mesa and LLVM,
    // tens of MiB for a window of charts. QT_QUICK_BACKEND still overrides.
    if (qEnvironmentVariableIsEmpty("QT_QUICK_BACKEND") && !atlas_settings_gpu_rendering()) {
        QQuickWindow::setGraphicsApi(QSGRendererInterface::Software);
    }

    QApplication app(argc, argv);
    // The display name, the window icon, and what Telamon.Ui's TelamonApp shows.
    telamon_app_ready();

    // Before the single-instance check, so --help and --version answer
    // here; --page goes to the running window if there is one.
    QCommandLineParser parser;
    parser.addHelpOption();
    parser.addVersionOption();
    const QCommandLineOption page = pageOptionSpec();
    parser.addOption(page);
    parser.process(app);

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
        {"system", made.system},
        {"hardware", made.hardware},
    };
    ThemeIcons themeIcons;
    QQmlEngine::setObjectOwnership(&themeIcons, QQmlEngine::CppOwnership);
    QVariantMap initial;
    initial.insert(QStringLiteral("themeIcons"), QVariant::fromValue(static_cast<QObject *>(&themeIcons)));
    Platform platform;
    QQmlEngine::setObjectOwnership(&platform, QQmlEngine::CppOwnership);
    initial.insert(QStringLiteral("platform"), QVariant::fromValue(static_cast<QObject *>(&platform)));
    initial.insert(QStringLiteral("startPage"), pageOption(parser, page));
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
            closeOnQuitSignals(window);
            QObject::connect(&service, &KDBusService::activateRequested, window, [window](const QStringList &args, const QString &) {
                // The second launch's arguments, its own name first. A
                // --page among them opens that page; parse() never exits.
                QCommandLineParser second;
                second.addHelpOption();
                second.addVersionOption();
                const QCommandLineOption page = pageOptionSpec();
                second.addOption(page);
                if (!args.isEmpty() && second.parse(args)) {
                    const QString name = pageOption(second, page);
                    if (!name.isEmpty()) {
                        QMetaObject::invokeMethod(window, "openPage", Q_ARG(QVariant, name));
                    }
                }
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
    // The window's last saves (its size, the page) are written on a thread
    // of their own: wait for them.
    atlas_settings_flush();
    return rc;
}

#include "main.moc"
