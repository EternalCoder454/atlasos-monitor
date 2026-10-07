// Prototype of Telamon.Ui's LiveChart, for the chart decision: a QQuickPaintedItem
// that draws a 60-sample history the way the Go Atlas Monitor's graph package
// does (caption bands, square grid, fill and line, border). It reads its
// samples from a Series, which stands in for the Rust ring buffer.
#pragma once

#include <QColor>
#include <QPointer>
#include <QQuickPaintedItem>
#include <QStaticText>
#include <QtQml/qqmlregistration.h>

#include <array>

// A fixed-size ring buffer of samples. In the app this is the Rust Series
// (CXX-Qt); the chart only needs copyInto(), max() and the changed signal.
class Series : public QObject
{
    Q_OBJECT
    QML_ELEMENT
    // The samples oldest first, as the Rust Series would publish them each
    // tick (the generic interface a shared Telamon.Ui chart can take).
    Q_PROPERTY(QList<qreal> values READ values NOTIFY changed)
public:
    static constexpr int Capacity = 60;
    using QObject::QObject;

    void push(double v);
    // Copies the samples oldest first into out and returns how many there are.
    int copyInto(double *out) const;
    double max() const;
    QList<qreal> values() const { return m_values; }

Q_SIGNALS:
    void changed();

private:
    QList<qreal> m_values;
    std::array<double, Capacity> m_buf{};
    int m_head = 0; // next write position
    int m_len = 0;
};

class LiveChart : public QQuickPaintedItem
{
    Q_OBJECT
    QML_ELEMENT
    Q_PROPERTY(Series *series READ series WRITE setSeries NOTIFY seriesChanged)
    Q_PROPERTY(QList<qreal> values READ values WRITE setValues NOTIFY valuesChanged)
    Q_PROPERTY(QString label MEMBER m_label NOTIFY labelChanged)
    Q_PROPERTY(QColor color MEMBER m_color NOTIFY colorChanged)
    Q_PROPERTY(QColor textColor MEMBER m_textColor NOTIFY colorChanged)
    Q_PROPERTY(bool percent MEMBER m_percent NOTIFY styleChanged)
    Q_PROPERTY(bool captions MEMBER m_captions NOTIFY styleChanged)
    // When set, the chart fills its own rectangle with this colour and is
    // opaque, so the software renderer copies it instead of blending it over
    // what is below (and skips drawing what it covers).
    Q_PROPERTY(QColor background READ background WRITE setBackground)
public:
    explicit LiveChart(QQuickItem *parent = nullptr);

    Series *series() const { return m_series; }
    void setSeries(Series *s);
    QList<qreal> values() const { return m_values; }
    void setValues(const QList<qreal> &v);
    QColor background() const { return fillColor(); }
    void setBackground(const QColor &c)
    {
        setFillColor(c);
        setOpaquePainting(c.isValid() && c.alpha() == 255);
    }

    void paint(QPainter *p) override;

    // Totals over every LiveChart in the process, for the benchmark.
    static qint64 s_paintNanos;
    static qint64 s_paints;

Q_SIGNALS:
    void seriesChanged();
    void valuesChanged();
    void labelChanged();
    void colorChanged();
    void styleChanged();

private:
    void drawText(QPainter *p, QStaticText &t, QString &shown, const QString &text, QPointF at, double alpha);

    QPointer<Series> m_series;
    QList<qreal> m_values;
    QString m_label;
    QColor m_color{0x39, 0xb8, 0xe3};
    QColor m_textColor{0xfc, 0xfc, 0xfc};
    bool m_percent = true;
    bool m_captions = true;

    std::array<double, Series::Capacity> m_scratch{};
    QStaticText m_labelText, m_valueText, m_peakText, m_spanText, m_zeroText;
    QString m_shownLabel, m_shownValue, m_shownPeak, m_shownSpan, m_shownZero;
};
