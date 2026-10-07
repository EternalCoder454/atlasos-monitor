// CoreGrid: each logical processor's load as a grid of cells, "Core 3" and
// its percent over a slim bar, as the Go version's CPU page. The columns
// follow the width: as many cells of at least minimumCellWidth as fit. Drawn
// with QPainter in one item, so a 32-thread machine repaints one image a
// tick rather than a hundred and more small items, and it renders on Qt
// Quick's software backend.
#pragma once

#include <QColor>
#include <QFont>
#include <QList>
#include <QQuickPaintedItem>
#include <QStaticText>
#include <QTransform>
#include <QtQml/qqmlregistration.h>

class CoreGrid : public QQuickPaintedItem
{
    Q_OBJECT
    QML_ELEMENT

    // Each processor's load, in processor order.
    Q_PROPERTY(QList<qreal> values READ values WRITE setValues NOTIFY valuesChanged)
    // The load of a full bar.
    Q_PROPERTY(qreal maximum MEMBER m_maximum NOTIFY styleChanged)
    // The bars' colour, and the text's (the track is the text's, faint).
    Q_PROPERTY(QColor color MEMBER m_color NOTIFY styleChanged)
    Q_PROPERTY(QColor textColor MEMBER m_textColor NOTIFY styleChanged)
    Q_PROPERTY(QFont font READ font WRITE setFont NOTIFY styleChanged)
    // A cell's name, with %1 for its number: "Core %1", translated.
    Q_PROPERTY(QString labelFormat READ labelFormat WRITE setLabelFormat NOTIFY styleChanged)
    Q_PROPERTY(qreal minimumCellWidth READ minimumCellWidth WRITE setMinimumCellWidth NOTIFY styleChanged)
    Q_PROPERTY(qreal columnSpacing READ columnSpacing WRITE setColumnSpacing NOTIFY styleChanged)
    Q_PROPERTY(qreal rowSpacing READ rowSpacing WRITE setRowSpacing NOTIFY styleChanged)
    // Right to left: the first cell at the right, names at the right of
    // their cells, bars filling leftwards.
    Q_PROPERTY(bool mirrored MEMBER m_mirrored NOTIFY styleChanged)
    // How many columns the width holds now.
    Q_PROPERTY(int columns READ columns NOTIFY columnsChanged)

public:
    explicit CoreGrid(QQuickItem *parent = nullptr);

    QList<qreal> values() const { return m_values; }
    void setValues(const QList<qreal> &values);
    QFont font() const { return m_font; }
    void setFont(const QFont &font);
    QString labelFormat() const { return m_labelFormat; }
    void setLabelFormat(const QString &format);
    qreal minimumCellWidth() const { return m_minimumCellWidth; }
    void setMinimumCellWidth(qreal w);
    qreal columnSpacing() const { return m_columnSpacing; }
    void setColumnSpacing(qreal s);
    qreal rowSpacing() const { return m_rowSpacing; }
    void setRowSpacing(qreal s);
    int columns() const { return m_columns; }

    void paint(QPainter *p) override;

Q_SIGNALS:
    void valuesChanged();
    void styleChanged();
    void columnsChanged();

protected:
    void itemChange(ItemChange change, const ItemChangeData &value) override;
    void geometryChange(const QRectF &newGeometry, const QRectF &oldGeometry) override;

private:
    // Columns and implicit height from the width, the count and the font.
    void relayout();
    double cellHeight() const;

    QList<qreal> m_values;
    qreal m_maximum = 100;
    QColor m_color = QColor(0x39, 0xb8, 0xe3);
    QColor m_textColor = Qt::black;
    QFont m_font;
    QString m_labelFormat = QStringLiteral("Core %1");
    qreal m_minimumCellWidth = 108;
    qreal m_columnSpacing = 12;
    qreal m_rowSpacing = 8;
    bool m_mirrored = false;
    int m_columns = 1;
    // The names, laid out once for the painter's transform: they change
    // only with the count, the font or the scale.
    QList<QStaticText> m_labels;
    QTransform m_labelTransform;
};
