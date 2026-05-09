package components

import (
	"fmt"
	"sync"
)

// ImageComponent renders an inline image in the terminal using Kitty or iTerm2 protocol.
// When no image protocol is available, it renders a text fallback.
type ImageComponent struct {
	base64Data string
	mimeType   string
	dims       ImageDimensions
	theme      ImageTheme
	opts       ImageOptions
	imageID    int // Kitty image ID for reuse/replacement

	cachedLines []string
	cachedWidth int
	mu          sync.Mutex
}

// ImageTheme configures colors for the image fallback text.
type ImageTheme struct {
	FallbackColor func(string) string
}

// ImageOptions configures image rendering.
type ImageOptions struct {
	MaxWidthCells  int
	MaxHeightCells int
	Filename       string
	ImageID        int // existing Kitty image ID to reuse
}

// NewImage creates a new image component.
func NewImage(base64Data, mimeType string, theme ImageTheme, opts ImageOptions) *ImageComponent {
	dims := GetImageDimensions(base64Data, mimeType)
	if dims == nil {
		dims = &ImageDimensions{WidthPx: 800, HeightPx: 600}
	}
	return &ImageComponent{
		base64Data: base64Data,
		mimeType:   mimeType,
		theme:      theme,
		opts:       opts,
		dims:       *dims,
		imageID:    opts.ImageID,
	}
}

// GetImageID returns the Kitty image ID used by this component.
func (img *ImageComponent) GetImageID() int {
	return img.imageID
}

// Invalidate clears the render cache so the next Render recalculates.
func (img *ImageComponent) Invalidate() {
	img.mu.Lock()
	img.cachedLines = nil
	img.cachedWidth = 0
	img.mu.Unlock()
}

// Render returns the lines to display for this image.
// For image protocols: renders empty lines for image height with the
// image sequence on the last line with proper cursor positioning.
// For no image support: renders a text fallback.
func (img *ImageComponent) Render(width int) []string {
	img.mu.Lock()
	defer img.mu.Unlock()

	if img.cachedLines != nil && img.cachedWidth == width {
		return img.cachedLines
	}

	maxWidth := width - 2
	if img.opts.MaxWidthCells > 0 && img.opts.MaxWidthCells < maxWidth {
		maxWidth = img.opts.MaxWidthCells
	}
	if maxWidth < 10 {
		maxWidth = 60
	}

	protocol := DetectImageCapability()

	if protocol != "" {
		if protocol == ImageProtocolKitty && img.imageID == 0 {
			img.imageID = allocateImageID()
		}

		result := RenderImage(img.base64Data, img.dims, ImageRenderOptions{
			MaxWidthCells:       maxWidth,
			MaxHeightCells:      img.opts.MaxHeightCells,
			PreserveAspectRatio: true,
			ImageID:             img.imageID,
			MoveCursor:          false,
		})

		if result != nil {
			if result.ImageID != 0 {
				img.imageID = result.ImageID
			}

			lines := make([]string, 0, result.Rows)
			for i := 0; i < result.Rows-1; i++ {
				lines = append(lines, "")
			}
			rowOffset := result.Rows - 1
			moveUp := ""
			moveDown := ""
			if rowOffset > 0 {
				moveUp = fmt.Sprintf("\x1b[%dA", rowOffset)
				if protocol == ImageProtocolKitty {
					moveDown = fmt.Sprintf("\x1b[%dB", rowOffset)
				}
			}
			lines = append(lines, moveUp+result.Sequence+moveDown)
			img.cachedLines = lines
			img.cachedWidth = width
			return lines
		}
	}

	// Fallback: text representation
	fallback := ImageFallback(img.mimeType, &img.dims, img.opts.Filename)
	line := fallback
	if img.theme.FallbackColor != nil {
		line = img.theme.FallbackColor(fallback)
	}
	img.cachedLines = []string{line}
	img.cachedWidth = width
	return img.cachedLines
}

// allocateImageID generates a random image ID for Kitty graphics protocol.
func allocateImageID() int {
	// Simple sequential ID generation; can be made random if needed
	return int(fastRand()%0xfffffffe) + 1
}

var randState uint32 = 1

func fastRand() uint32 {
	randState = randState*1103515245 + 12345
	return randState
}
