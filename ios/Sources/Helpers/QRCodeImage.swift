import CoreImage
import CoreImage.CIFilterBuiltins
import UIKit

enum QRCodeImage {
    static func make(from text: String, scale: CGFloat = 10) -> UIImage? {
        let data = Data(text.utf8)
        let filter = CIFilter.qrCodeGenerator()
        filter.setValue(data, forKey: "inputMessage")
        guard var output = filter.outputImage else { return nil }
        output = output.transformed(by: CGAffineTransform(scaleX: scale, y: scale))
        let context = CIContext()
        guard let cgImage = context.createCGImage(output, from: output.extent) else { return nil }
        return UIImage(cgImage: cgImage)
    }
}
