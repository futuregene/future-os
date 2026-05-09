package utils

import (
	"os"
	"path/filepath"
	"strings"
)

// DetectImageMimeType checks if a file at the given path is a supported image format
// by reading the file header (magic bytes). Returns the MIME type or empty string.
func DetectImageMimeType(path string) (string, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return "", err
	}
	return DetectImageMimeTypeFromBytes(data), nil
}

// DetectImageMimeTypeFromBytes checks magic bytes and returns the MIME type or empty string.
func DetectImageMimeTypeFromBytes(data []byte) string {
	if len(data) < 8 {
		return ""
	}

	// PNG: 89 50 4E 47
	if data[0] == 0x89 && data[1] == 0x50 && data[2] == 0x4E && data[3] == 0x47 {
		return "image/png"
	}

	// JPEG: FF D8 FF
	if data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
		return "image/jpeg"
	}

	// GIF: GIF87a or GIF89a
	if len(data) >= 6 && (string(data[0:6]) == "GIF87a" || string(data[0:6]) == "GIF89a") {
		return "image/gif"
	}

	// WebP: RIFF....WEBP
	if len(data) >= 12 && string(data[0:4]) == "RIFF" && string(data[8:12]) == "WEBP" {
		return "image/webp"
	}

	// BMP: BM
	if data[0] == 'B' && data[1] == 'M' {
		return "image/bmp"
	}

	// SVG: check extension as SVG has no reliable magic bytes
	return ""
}

// DetectImageMimeTypeFromExtension uses file extension as fallback.
func DetectImageMimeTypeFromExtension(path string) string {
	ext := strings.ToLower(filepath.Ext(path))
	switch ext {
	case ".png":
		return "image/png"
	case ".jpg", ".jpeg":
		return "image/jpeg"
	case ".gif":
		return "image/gif"
	case ".webp":
		return "image/webp"
	case ".bmp":
		return "image/bmp"
	case ".svg":
		return "image/svg+xml"
	case ".tiff", ".tif":
		return "image/tiff"
	case ".ico":
		return "image/x-icon"
	default:
		return ""
	}
}

// IsImageFile checks if a file path likely points to an image.
func IsImageFile(path string) bool {
	if DetectImageMimeTypeFromExtension(path) != "" {
		return true
	}
	mime, err := DetectImageMimeType(path)
	return err == nil && mime != ""
}
