// Chart benchmark: a window of N LiveCharts fed once per interval, measured
// in-process after a warm-up. Prints one JSON line and exits.
//   chartbench [--charts N] [--cols C] [--width W] [--height H]
//              [--interval MS] [--warmup S] [--seconds S]
//              [--idle] [--no-captions] [--gpu] [--grab FILE]
#include "livechart.h"

#include <QCommandLineParser>
#include <QElapsedTimer>
#include <QFile>
#include <QGuiApplication>
#include <QImage>
#include <QPainter>
#include <QQmlApplicationEngine>
#include <QQuickWindow>
#include <QRandomGenerator>
#include <QScreen>
#include <QtMath>
#include <QTimer>

#include <cstdio>
#include <ctime>

static double cpuMs()
{
    timespec ts;
    clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &ts);
    return ts.tv_sec * 1e3 + ts.tv_nsec / 1e6;
}

static qint64 procField(const char *path, const QByteArray &key)
{
    QFile f(QString::fromLatin1(path));
    if (!f.open(QIODevice::ReadOnly)) {
        return -1;
    }
    for (const QByteArray &line : f.readAll().split('\n')) {
        if (line.startsWith(key)) {
            return line.mid(key.size()).trimmed().split(' ').value(0).toLongLong();
        }
    }
    return -1;
}

int main(int argc, char *argv[])
{
    QCommandLineParser args;
    const QCommandLineOption charts(QStringLiteral("charts"), {}, QStringLiteral("n"), QStringLiteral("12"));
    const QCommandLineOption cols(QStringLiteral("cols"), {}, QStringLiteral("n"), QStringLiteral("2"));
    const QCommandLineOption width(QStringLiteral("width"), {}, QStringLiteral("px"), QStringLiteral("1280"));
    const QCommandLineOption height(QStringLiteral("height"), {}, QStringLiteral("px"), QStringLiteral("900"));
    const QCommandLineOption interval(QStringLiteral("interval"), {}, QStringLiteral("ms"), QStringLiteral("1000"));
    const QCommandLineOption warmup(QStringLiteral("warmup"), {}, QStringLiteral("s"), QStringLiteral("5"));
    const QCommandLineOption seconds(QStringLiteral("seconds"), {}, QStringLiteral("s"), QStringLiteral("30"));
    const QCommandLineOption idle(QStringLiteral("idle"));
    const QCommandLineOption noCaptions(QStringLiteral("no-captions"));
    const QCommandLineOption gpu(QStringLiteral("gpu"));
    const QCommandLineOption grab(QStringLiteral("grab"), {}, QStringLiteral("file"));
    const QCommandLineOption threadpool(QStringLiteral("threadpool"));
    const QCommandLineOption useList(QStringLiteral("list"));
    const QCommandLineOption opaque(QStringLiteral("opaque"));
    const QCommandLineOption seam(QStringLiteral("seam"), {}, QStringLiteral("prefix"));
    const QCommandLineOption micro(QStringLiteral("micro"), {}, QStringLiteral("paints"));
    const QCommandLineOption dpr(QStringLiteral("dpr"), {}, QStringLiteral("ratio"), QStringLiteral("1"));
    args.addOptions({charts, cols, width, height, interval, warmup, seconds, idle, noCaptions, gpu, grab, micro, dpr, threadpool, useList, opaque, seam});

    // Parsed before QGuiApplication, which needs the graphics API set first.
    QStringList argList;
    for (int i = 0; i < argc; ++i) {
        argList << QString::fromLocal8Bit(argv[i]);
    }
    args.process(argList);
    // Qt's raster engine hands every fill of 96+ spans to a thread pool and
    // waits; for chart-sized shapes the hand-off costs more than the fill.
    if (!args.isSet(threadpool)) {
        qputenv("QT_NO_GUI_THREADPOOL", "1");
    }
    if (!args.isSet(gpu)) {
        QQuickWindow::setGraphicsApi(QSGRendererInterface::Software);
    }

    QGuiApplication app(argc, argv);

    if (args.isSet(micro)) {
        // One chart painted into an image in a loop, as the software backend's
        // painter node does (fill, then paint), new sample before each paint.
        const double ratio = args.value(dpr).toDouble();
        Series s;
        LiveChart chart;
        chart.setSize(QSizeF(args.value(width).toInt(), args.value(height).toInt()));
        chart.setSeries(&s);
        chart.setProperty("label", QStringLiteral("CPU"));
        QImage img((chart.size() * ratio).toSize(), QImage::Format_ARGB32_Premultiplied);
        img.setDevicePixelRatio(ratio);
        double v = 50;
        QRandomGenerator seeded(42); // same data in every run, for pixel diffs
        auto *rng = &seeded;
        const int paints = args.value(micro).toInt();
        for (int i = 0; i < paints + 200; ++i) {
            if (i == 200) {
                LiveChart::s_paintNanos = 0;
                LiveChart::s_paints = 0;
            }
            v = std::clamp(v + rng->bounded(20.0) - 10.0, 0.0, 100.0);
            s.push(v);
            img.fill(Qt::transparent);
            QPainter p(&img);
            chart.paint(&p);
        }
        std::printf("{\"micro\":true,\"dpr\":%.2f,\"w\":%d,\"h\":%d,\"paint_us_each\":%.1f}\n", ratio,
                    img.width(), img.height(), LiveChart::s_paintNanos / 1e3 / LiveChart::s_paints);
        if (args.isSet(grab)) {
            img.save(args.value(grab));
        }
        return 0;
    }

    const int n = args.value(charts).toInt();
    QList<QObject *> series;
    auto *rng = QRandomGenerator::global();
    std::vector<double> level(n);
    for (int i = 0; i < n; ++i) {
        auto *s = new Series(&app);
        level[i] = rng->bounded(100.0);
        for (int k = 0; k < Series::Capacity; ++k) {
            s->push(i % 2 ? level[i] * 2e5 : level[i]);
        }
        series << s;
    }

    QQmlApplicationEngine engine;
    engine.setInitialProperties({
        {QStringLiteral("seriesList"), QVariant::fromValue(series)},
        {QStringLiteral("columns"), args.value(cols).toInt()},
        {QStringLiteral("width"), args.value(width).toInt()},
        {QStringLiteral("height"), args.value(height).toInt()},
        {QStringLiteral("captions"), !args.isSet(noCaptions)},
        {QStringLiteral("useList"), args.isSet(useList)},
        {QStringLiteral("opaque"), args.isSet(opaque)},
    });
    engine.loadFromModule(QStringLiteral("ChartBench"), QStringLiteral("Main"));
    auto *window = qobject_cast<QQuickWindow *>(engine.rootObjects().value(0));
    if (!window) {
        return 1;
    }

    // Frame timing: sync (where QQuickPaintedItem paints on the software
    // backend), render, and the whole frame up to the swap/flush.
    QElapsedTimer frameTimer;
    qint64 syncNs = 0, renderNs = 0, frameNs = 0, frames = 0, renderStart = 0;
    bool measuring = false;
    QObject::connect(window, &QQuickWindow::beforeSynchronizing, window, [&] { frameTimer.start(); }, Qt::DirectConnection);
    QObject::connect(window, &QQuickWindow::afterSynchronizing, window, [&] {
        if (measuring) syncNs += frameTimer.nsecsElapsed();
    }, Qt::DirectConnection);
    QObject::connect(window, &QQuickWindow::beforeRendering, window, [&] { renderStart = frameTimer.nsecsElapsed(); }, Qt::DirectConnection);
    QObject::connect(window, &QQuickWindow::afterRendering, window, [&] {
        if (measuring) renderNs += frameTimer.nsecsElapsed() - renderStart;
    }, Qt::DirectConnection);
    QObject::connect(window, &QQuickWindow::frameSwapped, window, [&] {
        if (measuring && frameTimer.isValid()) {
            frameNs += frameTimer.nsecsElapsed();
            ++frames;
        }
    }, Qt::DirectConnection);

    QTimer tick;
    tick.setInterval(args.value(interval).toInt());
    QObject::connect(&tick, &QTimer::timeout, [&] {
        for (int i = 0; i < n; ++i) {
            level[i] = std::clamp(level[i] + rng->bounded(20.0) - 10.0, 0.0, 100.0);
            static_cast<Series *>(series[i])->push(i % 2 ? level[i] * 2e5 : level[i]);
        }
    });
    if (!args.isSet(idle)) {
        tick.start();
    }

    double cpu0 = 0;
    qint64 syscr0 = 0;
    QTimer::singleShot(args.value(warmup).toInt() * 1000, [&] {
        cpu0 = cpuMs();
        syscr0 = procField("/proc/self/io", "syscr:");
        LiveChart::s_paintNanos = 0;
        LiveChart::s_paints = 0;
        measuring = true;
    });
    const int total = (args.value(warmup).toInt() + args.value(seconds).toInt()) * 1000;
    QTimer::singleShot(total, [&] {
        measuring = false;
        const double secs = args.value(seconds).toDouble();
        const double cpu = cpuMs() - cpu0;
        const qint64 syscr = procField("/proc/self/io", "syscr:") - syscr0;
        std::printf("{\"backend\":\"%s\",\"dpr\":%.2f,\"charts\":%d,\"interval\":%d,\"idle\":%s,"
                    "\"cpu_ms_s\":%.3f,\"paint_ms_s\":%.3f,\"paints\":%lld,\"paint_us_each\":%.1f,"
                    "\"frames\":%lld,\"sync_ms_s\":%.3f,\"render_ms_s\":%.3f,\"frame_ms_s\":%.3f,"
                    "\"frame_ms_each\":%.3f,\"syscr_s\":%.1f,\"rss_kib\":%lld,\"pss_kib\":%lld}\n",
                    args.isSet(gpu) ? "gpu" : "software", window->effectiveDevicePixelRatio(), n,
                    args.value(interval).toInt(), args.isSet(idle) ? "true" : "false", cpu / secs,
                    LiveChart::s_paintNanos / 1e6 / secs, LiveChart::s_paints,
                    LiveChart::s_paints ? LiveChart::s_paintNanos / 1e3 / LiveChart::s_paints : 0.0, frames,
                    syncNs / 1e6 / secs, renderNs / 1e6 / secs, frameNs / 1e6 / secs,
                    frames ? frameNs / 1e6 / frames : 0.0, syscr / secs,
                    procField("/proc/self/status", "VmRSS:"), procField("/proc/self/smaps_rollup", "Pss:"));
        std::fflush(stdout);
        if (args.isSet(grab)) {
            window->grabWindow().save(args.value(grab));
        }
        if (args.isSet(seam)) {
            // What is on screen (the offscreen platform hands back its backing
            // store, built up by partial updates) next to a full fresh render.
            // Ticks stop first and one more frame settles, so both show the
            // same samples.
            tick.stop();
            QTimer::singleShot(500, [&] {
                window->screen()->grabWindow(window->winId()).save(args.value(seam) + QStringLiteral("-screen.png"));
                window->grabWindow().save(args.value(seam) + QStringLiteral("-fresh.png"));
                QCoreApplication::quit();
            });
            return;
        }
        QCoreApplication::quit();
    });

    return app.exec();
}
