import SwiftUI
import PhotosUI
import UIKit
import UniformTypeIdentifiers

struct ChatInputBar: View {
    @Binding var messageText: String
    @Binding var selectedPhotoItems: [PhotosPickerItem]
    @Binding var stagedMedia: [StagedMediaItem]
    let showAttachButton: Bool
    let showMicButton: Bool
    @Binding var isInputFocused: Bool
    let onSend: () -> Void
    let onStartVoiceRecording: () -> Void
    let onChooseFile: () -> Void
    let onCreatePoll: () -> Void
    var onImagePaste: ((Data, String) -> Void)? = nil

    @State private var showPhotoPicker = false

    var body: some View {
        VStack(spacing: 0) {
            // Staging area for selected media
            if !stagedMedia.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(stagedMedia) { item in
                            StagedMediaThumbnail(item: item) {
                                withAnimation {
                                    stagedMedia.removeAll { $0.id == item.id }
                                }
                            }
                        }
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                }
            }

            HStack(alignment: .bottom, spacing: 8) {
                if showAttachButton {
                    Menu {
                        Button {
                            showPhotoPicker = true
                        } label: {
                            Label("Photos & Videos", systemImage: "photo.on.rectangle")
                        }

                        Button {
                            onChooseFile()
                        } label: {
                            Label("File", systemImage: "doc")
                        }

                        Button {
                            onCreatePoll()
                        } label: {
                            Label("Poll", systemImage: "chart.bar")
                        }
                    } label: {
                        Image(systemName: "plus")
                            .font(.body.weight(.semibold))
                            .frame(width: 52, height: 52)
                    }
                    .tint(.secondary)
                    .modifier(GlassCircleModifier())
                    .photosPicker(
                        isPresented: $showPhotoPicker,
                        selection: $selectedPhotoItems,
                        maxSelectionCount: 32,
                        matching: .any(of: [.images, .videos])
                    )
                }

                HStack(spacing: 10) {
                    StickerAwareTextView(
                        text: $messageText,
                        isFocused: $isInputFocused,
                        maxHeight: 150,
                        onSend: onSend,
                        onImagePaste: onImagePaste
                    )
                    .frame(minHeight: 36)
                    .overlay(alignment: .topLeading) {
                        if messageText.isEmpty {
                            Text("Message")
                                .foregroundStyle(.tertiary)
                                .padding(.leading, 5)
                                .padding(.top, 8)
                                .allowsHitTesting(false)
                        }
                    }
                    .accessibilityIdentifier(TestIds.chatMessageInput)

                    let isEmpty = messageText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                        && stagedMedia.isEmpty
                    if isEmpty, showMicButton {
                        Button {
                            onStartVoiceRecording()
                        } label: {
                            Image(systemName: "mic.fill")
                                .font(.title2)
                        }
                        .transition(.scale.combined(with: .opacity))
                    } else {
                        Button(action: { onSend() }) {
                            Image(systemName: "arrow.up.circle.fill")
                                .font(.title2)
                        }
                        .disabled(isEmpty)
                        .accessibilityIdentifier(TestIds.chatSend)
                        .transition(.scale.combined(with: .opacity))
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .animation(.easeInOut(duration: 0.15), value: messageText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && stagedMedia.isEmpty)
                .modifier(GlassInputModifier())
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 12)
        .padding(.vertical, 12)
    }
}

// MARK: - Staged Media Types

struct StagedMediaItem: Identifiable {
    let id: String
    let data: Data
    let filename: String
    let mimeType: String
    let thumbnail: UIImage?
}

private struct StagedMediaThumbnail: View {
    let item: StagedMediaItem
    let onRemove: () -> Void

    var body: some View {
        ZStack(alignment: .topTrailing) {
            if let thumb = item.thumbnail {
                Image(uiImage: thumb)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
                    .frame(width: 64, height: 64)
                    .clipShape(RoundedRectangle(cornerRadius: 8))
            } else {
                RoundedRectangle(cornerRadius: 8)
                    .fill(Color.gray.opacity(0.3))
                    .frame(width: 64, height: 64)
                    .overlay {
                        Image(systemName: "doc")
                            .foregroundStyle(.secondary)
                    }
            }

            Button(action: onRemove) {
                Image(systemName: "xmark.circle.fill")
                    .font(.caption)
                    .foregroundStyle(.white, .black.opacity(0.6))
            }
            .offset(x: 4, y: -4)
        }
    }
}

// MARK: - Sticker-aware text view

/// A `UITextView` wrapper that intercepts image paste from sticker keyboards
/// and forwards image data via `onImagePaste`.
struct StickerAwareTextView: UIViewRepresentable {
    @Binding var text: String
    @Binding var isFocused: Bool
    var maxHeight: CGFloat = 150
    var onSend: (() -> Void)?
    var onImagePaste: ((Data, String) -> Void)?

    func makeCoordinator() -> Coordinator {
        Coordinator(parent: self)
    }

    func makeUIView(context: Context) -> PastableTextView {
        let tv = PastableTextView()
        tv.delegate = context.coordinator
        tv.pasteDelegate = context.coordinator
        tv.maxAllowedHeight = maxHeight
        tv.onImagePaste = { data, mime in
            onImagePaste?(data, mime)
        }
        tv.onReturnKey = { onSend?() }
        tv.font = .preferredFont(forTextStyle: .body)
        tv.backgroundColor = .clear
        tv.textContainerInset = UIEdgeInsets(top: 8, left: 0, bottom: 8, right: 0)
        tv.textContainer.lineFragmentPadding = 5
        tv.isScrollEnabled = false
        tv.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        tv.setContentHuggingPriority(.defaultHigh, for: .vertical)
        // Auto-focus on Mac Catalyst
        if ProcessInfo.processInfo.isiOSAppOnMac {
            DispatchQueue.main.async { tv.becomeFirstResponder() }
        }
        return tv
    }

    func updateUIView(_ uiView: PastableTextView, context: Context) {
        context.coordinator.parent = self

        if uiView.text != text {
            uiView.text = text
            uiView.recalculateHeight()
        }
        uiView.onImagePaste = { data, mime in
            onImagePaste?(data, mime)
        }
        uiView.onReturnKey = { onSend?() }

        if isFocused && !uiView.isFirstResponder {
            DispatchQueue.main.async {
                guard self.isFocused, !uiView.isFirstResponder else { return }
                uiView.becomeFirstResponder()
            }
        } else if !isFocused && uiView.isFirstResponder {
            DispatchQueue.main.async {
                guard !self.isFocused, uiView.isFirstResponder else { return }
                uiView.resignFirstResponder()
            }
        }
    }

    class Coordinator: NSObject, UITextViewDelegate, UITextPasteDelegate {
        var parent: StickerAwareTextView

        init(parent: StickerAwareTextView) {
            self.parent = parent
        }

        // MARK: - UITextPasteDelegate

        func textPasteConfigurationSupporting(
            _ textPasteConfigurationSupporting: UITextPasteConfigurationSupporting,
            transform item: UITextPasteItem
        ) {
            if item.itemProvider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
                item.itemProvider.loadDataRepresentation(forTypeIdentifier: UTType.image.identifier) { [weak self] data, _ in
                    guard let data = data else { return }
                    DispatchQueue.main.async {
                        guard let tv = textPasteConfigurationSupporting as? PastableTextView else { return }
                        tv.onImagePaste?(data, "image/png")
                        self?.stripAttachments(in: tv)
                    }
                }
                item.setNoResult()
                return
            }
            item.setDefaultResult()
        }

        // MARK: - UITextViewDelegate

        func textViewDidChange(_ textView: UITextView) {
            (textView as? PastableTextView)?.recalculateHeight()

            guard textView.text.contains("\u{FFFC}") else {
                parent.text = textView.text
                return
            }

            if let data = extractStickerImage(from: textView) {
                stripAttachments(in: textView)
                (textView as? PastableTextView)?.onImagePaste?(data, "image/png")
                return
            }

            parent.text = textView.text
        }

        // MARK: - Helpers

        /// Extracts image data from sticker content (NSAdaptiveImageGlyph or NSTextAttachment).
        private func extractStickerImage(from textView: UITextView) -> Data? {
            let storage = textView.textStorage
            let range = NSRange(location: 0, length: storage.length)

            // iOS 18+: NSAdaptiveImageGlyph (used by sticker keyboard)
            if #available(iOS 18.0, *) {
                var result: Data?
                storage.enumerateAttributes(in: range) { attrs, _, stop in
                    for (_, value) in attrs {
                        if let glyph = value as? NSAdaptiveImageGlyph,
                           let image = UIImage(data: glyph.imageContent),
                           let pngData = image.pngData() {
                            result = pngData
                            stop.pointee = true
                            return
                        }
                    }
                }
                if result != nil { return result }
            }

            // Fallback: NSTextAttachment
            var result: Data?
            storage.enumerateAttribute(.attachment, in: range) { value, _, stop in
                guard let attachment = value as? NSTextAttachment else { return }
                if let data = attachment.contents {
                    result = data
                } else if let image = attachment.image, let data = image.pngData() {
                    result = data
                } else if let fw = attachment.fileWrapper, let data = fw.regularFileContents {
                    result = data
                } else if let image = attachment.image(forBounds: attachment.bounds, textContainer: nil, characterIndex: 0),
                          let data = image.pngData() {
                    result = data
                }
                if result != nil { stop.pointee = true }
            }
            return result
        }

        /// Strips object-replacement characters and syncs the text binding.
        private func stripAttachments(in textView: UITextView) {
            let plain = textView.text.replacingOccurrences(of: "\u{FFFC}", with: "")
            textView.text = plain
            parent.text = plain
        }

        func textViewDidBeginEditing(_ textView: UITextView) {
            parent.isFocused = true
        }

        func textViewDidEndEditing(_ textView: UITextView) {
            parent.isFocused = false
        }
    }
}

/// UITextView subclass that intercepts paste, manages dynamic height, and detects image content.
class PastableTextView: UITextView {
    var onImagePaste: ((Data, String) -> Void)?
    var onReturnKey: (() -> Void)?
    var maxAllowedHeight: CGFloat = 150

    override var intrinsicContentSize: CGSize {
        let size = sizeThatFits(CGSize(width: bounds.width, height: .greatestFiniteMagnitude))
        let clamped = min(size.height, maxAllowedHeight)
        return CGSize(width: UIView.noIntrinsicMetric, height: clamped)
    }

    /// Recalculates intrinsic height and toggles scrolling when content exceeds max.
    func recalculateHeight() {
        let contentHeight = sizeThatFits(CGSize(width: bounds.width, height: .greatestFiniteMagnitude)).height
        let shouldScroll = contentHeight > maxAllowedHeight
        if isScrollEnabled != shouldScroll {
            isScrollEnabled = shouldScroll
        }
        invalidateIntrinsicContentSize()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        recalculateHeight()
    }

    override func paste(_ sender: Any?) {
        let pb = UIPasteboard.general

        // Check for GIF data first (preserves animation)
        if let gifData = pb.data(forPasteboardType: "com.compuserve.gif") {
            onImagePaste?(gifData, "image/gif")
            return
        }

        // Check for PNG data
        if let pngData = pb.data(forPasteboardType: "public.png") {
            onImagePaste?(pngData, "image/png")
            return
        }

        // Fallback: any image on pasteboard
        if pb.hasImages, let image = pb.image, let pngData = image.pngData() {
            onImagePaste?(pngData, "image/png")
            return
        }

        super.paste(sender)
    }

    override func canPerformAction(_ action: Selector, withSender sender: Any?) -> Bool {
        if action == #selector(paste(_:)) && UIPasteboard.general.hasImages {
            return true
        }
        return super.canPerformAction(action, withSender: sender)
    }

    override func pressesBegan(_ presses: Set<UIPress>, with event: UIPressesEvent?) {
        // Handle Return key to send (without Shift)
        if let key = presses.first?.key,
           key.keyCode == .keyboardReturnOrEnter,
           !key.modifierFlags.contains(.shift) {
            onReturnKey?()
            return
        }
        super.pressesBegan(presses, with: event)
    }
}

// MARK: - Glass modifiers (shared)

struct GlassInputModifier: ViewModifier {
    func body(content: Content) -> some View {
        #if compiler(>=6.2)
        if #available(iOS 26.0, *) {
            content
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .glassEffect(.regular.interactive(), in: RoundedRectangle(cornerRadius: 20))
        } else {
            content
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 20))
        }
        #else
        content
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 20))
        #endif
    }
}

struct GlassCircleModifier: ViewModifier {
    func body(content: Content) -> some View {
        #if compiler(>=6.2)
        if #available(iOS 26.0, *) {
            content
                .glassEffect(.regular.interactive(), in: Circle())
        } else {
            content
                .background(.ultraThinMaterial, in: Circle())
        }
        #else
        content
            .background(.ultraThinMaterial, in: Circle())
        #endif
    }
}

// MARK: - Previews

#if DEBUG
private struct ChatInputBarPreview: View {
    @State var messageText = ""
    @State var selectedPhotoItems: [PhotosPickerItem] = []
    @State var stagedMedia: [StagedMediaItem] = []
    @State var isInputFocused = false

    let showAttach: Bool
    let showMic: Bool

    var body: some View {
        ChatInputBar(
            messageText: $messageText,
            selectedPhotoItems: $selectedPhotoItems,
            stagedMedia: $stagedMedia,
            showAttachButton: showAttach,
            showMicButton: showMic,
            isInputFocused: $isInputFocused,
            onSend: {},
            onStartVoiceRecording: {},
            onChooseFile: {},
            onCreatePoll: {}
        )
    }
}

#Preview("Input Bar — Full") {
    VStack {
        Spacer()
        ChatInputBarPreview(showAttach: true, showMic: true)
    }
    .background(Color(uiColor: .systemBackground))
}

#Preview("Input Bar — No Attach") {
    VStack {
        Spacer()
        ChatInputBarPreview(showAttach: false, showMic: false)
    }
    .background(Color(uiColor: .systemBackground))
}

#Preview("Input Bar — With Text") {
    VStack {
        Spacer()
        ChatInputBarPreview(showAttach: true, showMic: true)
    }
    .background(Color(uiColor: .systemBackground))
    .onAppear {}
}
#endif
