import SwiftUI
import UIKit

struct ZoomableImageView: UIViewRepresentable {
    let image: UIImage?
    let onZoomScaleChange: (CGFloat) -> Void

    func makeUIView(context: Context) -> ZoomableScrollView {
        let view = ZoomableScrollView()
        view.onZoomScaleChange = onZoomScaleChange
        view.displayImage = image
        return view
    }

    func updateUIView(_ uiView: ZoomableScrollView, context: Context) {
        if uiView.displayImage !== image {
            uiView.displayImage = image
        }
    }
}

final class ZoomableScrollView: UIView, UIScrollViewDelegate {
    private let scrollView = UIScrollView()
    private let imageView = UIImageView()
    var onZoomScaleChange: ((CGFloat) -> Void)?

    var displayImage: UIImage? {
        didSet {
            imageView.image = displayImage
            setNeedsLayout()
        }
    }

    override init(frame: CGRect) {
        super.init(frame: frame)
        setup()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError()
    }

    private func setup() {
        backgroundColor = .clear

        scrollView.delegate = self
        scrollView.minimumZoomScale = 1.0
        scrollView.maximumZoomScale = 5.0
        scrollView.showsVerticalScrollIndicator = false
        scrollView.showsHorizontalScrollIndicator = false
        scrollView.bouncesZoom = true
        scrollView.isScrollEnabled = false
        scrollView.contentInsetAdjustmentBehavior = .never
        scrollView.backgroundColor = .clear

        imageView.contentMode = .scaleAspectFit
        imageView.clipsToBounds = true

        addSubview(scrollView)
        scrollView.addSubview(imageView)

        let doubleTap = UITapGestureRecognizer(
            target: self, action: #selector(handleDoubleTap(_:)))
        doubleTap.numberOfTapsRequired = 2
        scrollView.addGestureRecognizer(doubleTap)
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard !bounds.isEmpty else { return }
        let needsReset = scrollView.frame.size != bounds.size
        scrollView.frame = bounds
        if needsReset {
            imageView.frame = bounds
            scrollView.setZoomScale(1.0, animated: false)
            scrollView.isScrollEnabled = false
            onZoomScaleChange?(1.0)
        }
    }

    private func centerImageView() {
        let size = scrollView.bounds.size
        var frame = imageView.frame
        frame.origin.x = frame.width < size.width ? (size.width - frame.width) / 2 : 0
        frame.origin.y = frame.height < size.height ? (size.height - frame.height) / 2 : 0
        imageView.frame = frame
    }

    @objc private func handleDoubleTap(_ gesture: UITapGestureRecognizer) {
        if scrollView.zoomScale > scrollView.minimumZoomScale {
            scrollView.setZoomScale(scrollView.minimumZoomScale, animated: true)
        } else {
            let point = gesture.location(in: imageView)
            let scale: CGFloat = 3.0
            let size = CGSize(
                width: scrollView.bounds.width / scale,
                height: scrollView.bounds.height / scale)
            scrollView.zoom(
                to: CGRect(
                    x: point.x - size.width / 2,
                    y: point.y - size.height / 2,
                    width: size.width,
                    height: size.height),
                animated: true)
        }
    }

    // MARK: - UIScrollViewDelegate

    func viewForZooming(in scrollView: UIScrollView) -> UIView? {
        imageView
    }

    func scrollViewDidZoom(_ scrollView: UIScrollView) {
        centerImageView()
        updateScrollEnabled()
        onZoomScaleChange?(scrollView.zoomScale)
    }

    func scrollViewDidEndZooming(
        _ scrollView: UIScrollView, with view: UIView?, atScale scale: CGFloat
    ) {
        updateScrollEnabled()
        onZoomScaleChange?(scale)
    }

    private func updateScrollEnabled() {
        scrollView.isScrollEnabled = scrollView.zoomScale > scrollView.minimumZoomScale + 0.01
    }
}
