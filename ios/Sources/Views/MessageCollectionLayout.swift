import UIKit

struct MessageCollectionViewportMetrics: Equatable {
    let contentInset: UIEdgeInsets
    let scrollIndicatorInsets: UIEdgeInsets
    let jumpButtonBottomOffset: CGFloat
}

enum MessageCollectionUpdateKind: Equatable {
    case reconfigureOnly
    case tailMutation
    case structural
}

enum MessageCollectionLayout {
    static func viewportMetrics(
        extraBottomSpacing: CGFloat = 20,
        jumpButtonSpacing: CGFloat = 12
    ) -> MessageCollectionViewportMetrics {
        return MessageCollectionViewportMetrics(
            contentInset: UIEdgeInsets(top: 0, left: 0, bottom: extraBottomSpacing, right: 0),
            scrollIndicatorInsets: .zero,
            jumpButtonBottomOffset: jumpButtonSpacing
        )
    }

    static func classifyUpdate(oldIDs: [String], newIDs: [String]) -> MessageCollectionUpdateKind {
        guard oldIDs != newIDs else { return .reconfigureOnly }
        if oldIDs.isPrefix(of: newIDs) || newIDs.isPrefix(of: oldIDs) {
            return .tailMutation
        }
        return .structural
    }

    static func isNearBottom(
        contentOffsetY: CGFloat,
        boundsHeight: CGFloat,
        contentHeight: CGFloat,
        adjustedInsets: UIEdgeInsets,
        tolerance: CGFloat = 50
    ) -> Bool {
        let visibleBottom = contentOffsetY + boundsHeight - adjustedInsets.bottom
        return visibleBottom >= contentHeight - tolerance
    }

    static func bottomContentOffset(
        contentHeight: CGFloat,
        boundsHeight: CGFloat,
        adjustedInsets: UIEdgeInsets
    ) -> CGPoint {
        let minOffsetY = -adjustedInsets.top
        let maxOffsetY = max(minOffsetY, contentHeight - boundsHeight + adjustedInsets.bottom)
        return CGPoint(x: 0, y: maxOffsetY)
    }
}

private extension Array where Element: Equatable {
    func isPrefix(of other: [Element]) -> Bool {
        guard count <= other.count else { return false }
        return zip(self, other).allSatisfy(==)
    }
}
