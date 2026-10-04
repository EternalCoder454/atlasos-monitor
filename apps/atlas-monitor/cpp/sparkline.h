// Sparkline: the last minute of one figure, small enough for a list row: a
// line and a soft fill under it, no grid and no captions. The newest sample
// sits at the right edge; a NaN breaks the line. Drawn with QPainter, so it
// renders on Qt Quick's software backend, and it repaints only when the
// samples change: the Overview hands every row a fresh list each tick, and
// an idle device's flat line would otherwise repaint for nothing.
#pragma once

#include <QColor>
#include <QList>
#include <QQuickPaintedItem>
#include <QtQml/qqmlregistration.h>

class Sparkline : public QQuickPaintedItem
{
    Q_OBJECT
    QML_ELEMENT

    // The samples, oldest first; the last `length` are drawn.
    Q_PROPERTY(QList<qreal> values READ values WRITE setValues NOTIFY valuesChanged)
    // The value at the top. 0 scales to the samples: 1.25 times the highest
    // one shown, and at least minimumScale (a quiet drive's line stays low).
    Q_PROPERTY(qreal maximum MEMBER m_maximum NOTIFY styleChanged)
    Q_PROPERTY(qreal minimumScale MEMBER m_minimumScale NOTIFY styleChanged)
    Q_PROPERTY(QColor color MEMBER m_color NOTIFY styleChanged)
    // How many samples span the width.
    Q_PROPERTY(int length MEMBER m_length NOTIFY styleChanged)

public:
    explicit Sparkline(QQuickItem *parent = nullptr);

    QList<qreal> values() const { return m_values; }
    void setValues(const QList<qreal> &values);

    void paint(QPainter *p) override;

Q_SIGNALS:
    void valuesChanged();
    void styleChanged();

private:
    QList<qreal> m_values;
    qreal m_maximum = 100;
    qreal m_minimumScale = 0;
    QColor m_color = QColor(0x39, 0xb8, 0xe3);
    int m_length = 60;
};
