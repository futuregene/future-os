package components

import (
	"encoding/base64"
	"encoding/binary"
	"fmt"
	"math"
	"os"
	"strings"
)

// ImageProtocol represents a terminal image protocol.
type ImageProtocol string

const (
	ImageProtocolKitty ImageProtocol = "kitty"
	ImageProtocolITerm2 ImageProtocol = "iterm2"
)

// CellDimensions holds terminal cell pixel dimensions.
type CellDimensions struct {
	WidthPx  int
	HeightPx int
}

// ImageDimensions holds image pixel dimensions.
type ImageDimensions struct {
	WidthPx  int
	HeightPx int
}

// ImageRenderOptions configures image rendering.
type ImageRenderOptions struct {
	MaxWidthCells       int
	MaxHeightCells      int
	PreserveAspectRatio bool
	ImageID             int // Kitty image ID for reuse/replacement
	MoveCursor          bool // Kitty cursor movement after placement
}

// Default cell dimensions (updated when terminal responds to query).
var cellDimensions = CellDimensions{WidthPx: 9, HeightPx: 18}

// SetCellDimensions updates the terminal cell pixel dimensions.
func SetCellDimensions(dims CellDimensions) {
	cellDimensions = dims
}

// GetCellDimensions returns current cell dimensions.
func GetCellDimensions() CellDimensions {
	return cellDimensions
}

// DetectImageCapability checks the terminal environment for image protocol support.
// Returns the supported protocol or empty string if none.
func DetectImageCapability() ImageProtocol {
	termProgram := strings.ToLower(os.Getenv("TERM_PROGRAM"))
	term := strings.ToLower(os.Getenv("TERM"))

	// tmux/screen: no image support for safety
	if os.Getenv("TMUX") != "" || strings.HasPrefix(term, "tmux") || strings.HasPrefix(term, "screen") {
		return ""
	}

	if os.Getenv("KITTY_WINDOW_ID") != "" || termProgram == "kitty" {
		return ImageProtocolKitty
	}
	if termProgram == "ghostty" || strings.Contains(term, "ghostty") || os.Getenv("GHOSTTY_RESOURCES_DIR") != "" {
		return ImageProtocolKitty
	}
	if os.Getenv("WEZTERM_PANE") != "" || termProgram == "wezterm" {
		return ImageProtocolKitty
	}
	if os.Getenv("ITERM_SESSION_ID") != "" || strings.Contains(termProgram, "iterm") {
		return ImageProtocolITerm2
	}
	return ""
}

// EncodeKittyImage encodes base64 image data using Kitty graphics protocol.
// Supports chunked transmission for large images (>4096 bytes).
func EncodeKittyImage(base64Data string, columns, rows, imageID int, moveCursor bool) string {
	const chunkSize = 4096

	params := []string{"a=T", "f=100", "q=2"}
	if !moveCursor {
		params = append(params, "C=1")
	}
	if columns > 0 {
		params = append(params, fmt.Sprintf("c=%d", columns))
	}
	if rows > 0 {
		params = append(params, fmt.Sprintf("r=%d", rows))
	}
	if imageID > 0 {
		params = append(params, fmt.Sprintf("i=%d", imageID))
	}
	paramStr := strings.Join(params, ",")

	if len(base64Data) <= chunkSize {
		return fmt.Sprintf("\x1b_G%s;%s\x1b\\", paramStr, base64Data)
	}

	var chunks []string
	offset := 0
	isFirst := true

	for offset < len(base64Data) {
		end := offset + chunkSize
		if end > len(base64Data) {
			end = len(base64Data)
		}
		chunk := base64Data[offset:end]

		if isFirst {
			chunks = append(chunks, fmt.Sprintf("\x1b_G%s,m=1;%s\x1b\\", paramStr, chunk))
			isFirst = false
		} else if end >= len(base64Data) {
			chunks = append(chunks, fmt.Sprintf("\x1b_Gm=0;%s\x1b\\", chunk))
		} else {
			chunks = append(chunks, fmt.Sprintf("\x1b_Gm=1;%s\x1b\\", chunk))
		}
		offset = end
	}

	return strings.Join(chunks, "")
}

// DeleteKittyImage generates the sequence to delete a Kitty image by ID.
func DeleteKittyImage(imageID int) string {
	return fmt.Sprintf("\x1b_Ga=d,d=I,i=%d,q=2\x1b\\", imageID)
}

// DeleteAllKittyImages generates the sequence to delete all Kitty images.
func DeleteAllKittyImages() string {
	return "\x1b_Ga=d,d=A,q=2\x1b\\"
}

// EncodeITerm2Image encodes base64 image data using iTerm2 inline image protocol.
func EncodeITerm2Image(base64Data string, width int, height string, name string, preserveAspectRatio bool, inline bool) string {
	inlineVal := 1
	if !inline {
		inlineVal = 0
	}
	params := []string{fmt.Sprintf("inline=%d", inlineVal)}
	if width > 0 {
		params = append(params, fmt.Sprintf("width=%d", width))
	}
	if height != "" {
		params = append(params, fmt.Sprintf("height=%s", height))
	}
	if name != "" {
		nameB64 := base64.StdEncoding.EncodeToString([]byte(name))
		params = append(params, fmt.Sprintf("name=%s", nameB64))
	}
	if !preserveAspectRatio {
		params = append(params, "preserveAspectRatio=0")
	}
	return fmt.Sprintf("\x1b]1337;File=%s:%s\x07", strings.Join(params, ";"), base64Data)
}

// CalculateImageRows computes how many terminal rows the image will occupy.
func CalculateImageRows(dims ImageDimensions, targetWidthCells int, cellDims CellDimensions) int {
	targetWidthPx := targetWidthCells * cellDims.WidthPx
	scale := float64(targetWidthPx) / float64(dims.WidthPx)
	scaledHeightPx := int(math.Ceil(float64(dims.HeightPx) * scale))
	rows := (scaledHeightPx + cellDims.HeightPx - 1) / cellDims.HeightPx
	if rows < 1 {
		rows = 1
	}
	return rows
}

// GetPNGDimensions parses PNG dimensions from base64-encoded data.
func GetPNGDimensions(base64Data string) *ImageDimensions {
	data, err := base64.StdEncoding.DecodeString(base64Data)
	if err != nil || len(data) < 24 {
		return nil
	}
	// PNG signature
	if data[0] != 0x89 || data[1] != 0x50 || data[2] != 0x4e || data[3] != 0x47 {
		return nil
	}
	width := int(binary.BigEndian.Uint32(data[16:20]))
	height := int(binary.BigEndian.Uint32(data[20:24]))
	return &ImageDimensions{WidthPx: width, HeightPx: height}
}

// GetJPEGDimensions parses JPEG dimensions from base64-encoded data.
func GetJPEGDimensions(base64Data string) *ImageDimensions {
	data, err := base64.StdEncoding.DecodeString(base64Data)
	if err != nil || len(data) < 2 {
		return nil
	}
	if data[0] != 0xff || data[1] != 0xd8 {
		return nil
	}
	offset := 2
	for offset < len(data)-9 {
		if data[offset] != 0xff {
			offset++
			continue
		}
		marker := data[offset+1]
		if marker >= 0xc0 && marker <= 0xc2 {
			height := int(binary.BigEndian.Uint16(data[offset+5 : offset+7]))
			width := int(binary.BigEndian.Uint16(data[offset+7 : offset+9]))
			return &ImageDimensions{WidthPx: width, HeightPx: height}
		}
		if offset+3 >= len(data) {
			return nil
		}
		length := int(binary.BigEndian.Uint16(data[offset+2 : offset+4]))
		if length < 2 {
			return nil
		}
		offset += 2 + length
	}
	return nil
}

// GetGIFDimensions parses GIF dimensions from base64-encoded data.
func GetGIFDimensions(base64Data string) *ImageDimensions {
	data, err := base64.StdEncoding.DecodeString(base64Data)
	if err != nil || len(data) < 10 {
		return nil
	}
	sig := string(data[0:6])
	if sig != "GIF87a" && sig != "GIF89a" {
		return nil
	}
	width := int(binary.LittleEndian.Uint16(data[6:8]))
	height := int(binary.LittleEndian.Uint16(data[8:10]))
	return &ImageDimensions{WidthPx: width, HeightPx: height}
}

// GetWebPDimensions parses WebP dimensions from base64-encoded data.
func GetWebPDimensions(base64Data string) *ImageDimensions {
	data, err := base64.StdEncoding.DecodeString(base64Data)
	if err != nil || len(data) < 30 {
		return nil
	}
	riff := string(data[0:4])
	webp := string(data[8:12])
	if riff != "RIFF" || webp != "WEBP" {
		return nil
	}
	chunk := string(data[12:16])
	switch chunk {
	case "VP8 ":
		if len(data) < 30 {
			return nil
		}
		width := int(binary.LittleEndian.Uint16(data[26:28])) & 0x3fff
		height := int(binary.LittleEndian.Uint16(data[28:30])) & 0x3fff
		return &ImageDimensions{WidthPx: width, HeightPx: height}
	case "VP8L":
		if len(data) < 25 {
			return nil
		}
		bits := binary.LittleEndian.Uint32(data[21:25])
		width := int(bits&0x3fff) + 1
		height := int((bits>>14)&0x3fff) + 1
		return &ImageDimensions{WidthPx: width, HeightPx: height}
	case "VP8X":
		if len(data) < 30 {
			return nil
		}
		width := int(data[24]) | int(data[25])<<8 | int(data[26])<<16 + 1
		height := int(data[27]) | int(data[28])<<8 | int(data[29])<<16 + 1
		return &ImageDimensions{WidthPx: width, HeightPx: height}
	}
	return nil
}

// GetImageDimensions detects image dimensions from base64 data by mime type.
func GetImageDimensions(base64Data, mimeType string) *ImageDimensions {
	switch mimeType {
	case "image/png":
		return GetPNGDimensions(base64Data)
	case "image/jpeg":
		return GetJPEGDimensions(base64Data)
	case "image/gif":
		return GetGIFDimensions(base64Data)
	case "image/webp":
		return GetWebPDimensions(base64Data)
	}
	return nil
}

// RenderImageResult holds the result of rendering a terminal image.
type RenderImageResult struct {
	Sequence string // escape sequence to emit
	Rows     int    // number of terminal rows occupied
	ImageID  int    // Kitty image ID (0 if not applicable)
}

// RenderImage encodes a base64 image for display in the terminal.
// Returns nil if no image protocol is available.
func RenderImage(base64Data string, dims ImageDimensions, opts ImageRenderOptions) *RenderImageResult {
	protocol := DetectImageCapability()
	if protocol == "" {
		return nil
	}
	maxWidth := opts.MaxWidthCells
	if maxWidth <= 0 {
		maxWidth = 80
	}
	rows := CalculateImageRows(dims, maxWidth, GetCellDimensions())
	if protocol == ImageProtocolKitty {
		sequence := EncodeKittyImage(base64Data, maxWidth, rows, opts.ImageID, opts.MoveCursor)
		return &RenderImageResult{Sequence: sequence, Rows: rows, ImageID: opts.ImageID}
	}
	if protocol == ImageProtocolITerm2 {
		sequence := EncodeITerm2Image(base64Data, maxWidth, "auto", "", opts.PreserveAspectRatio, true)
		return &RenderImageResult{Sequence: sequence, Rows: rows}
	}
	return nil
}

// Hyperlink wraps text in an OSC 8 hyperlink sequence.
// Terminals that support OSC 8 will render it as a clickable link.
func Hyperlink(text, url string) string {
	return fmt.Sprintf("\x1b]8;;%s\x1b\\%s\x1b]8;;\x1b\\", url, text)
}

// ImageFallback generates a text representation when images can't be rendered.
func ImageFallback(mimeType string, dims *ImageDimensions, filename string) string {
	parts := []string{}
	if filename != "" {
		parts = append(parts, filename)
	}
	parts = append(parts, fmt.Sprintf("[%s]", mimeType))
	if dims != nil {
		parts = append(parts, fmt.Sprintf("%dx%d", dims.WidthPx, dims.HeightPx))
	}
	return fmt.Sprintf("[Image: %s]", strings.Join(parts, " "))
}

// IsImageLine checks if a rendered line contains an image escape sequence.
func IsImageLine(line string) bool {
	return strings.Contains(line, "\x1b_G") || strings.Contains(line, "\x1b]1337;File=")
}
