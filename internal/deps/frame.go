package deps

import (
	tea "github.com/charmbracelet/bubbletea"
)

// FrameModel is the smallest bubbletea model that proves what section 9 of
// this version assumes: a model renders a frame, its Update and View are
// ordinary functions that can be called with no terminal attached, and the
// program quits when the operator asks it to.
//
// It renders a string it is handed rather than deciding anything, because the
// deciding in this version belongs to the step log and not to the view over it.
// Section 9 grows this into the real interface.
type FrameModel struct {
	frame string
}

// NewFrameModel returns a model that renders frame until the operator quits.
func NewFrameModel(frame string) FrameModel {
	return FrameModel{frame: frame}
}

// Init starts no work. A frame has nothing to do until the operator presses a
// key.
func (m FrameModel) Init() tea.Cmd {
	return nil
}

// Update returns tea.Quit for the keys that mean stop, and the model unchanged
// for everything else. It holds no state beyond the frame, so there is nothing
// here for a terminal to be responsible for.
func (m FrameModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	if key, ok := msg.(tea.KeyMsg); ok {
		switch key.String() {
		case "q", "ctrl+c":
			return m, tea.Quit
		}
	}
	return m, nil
}

// View returns the frame. It is a plain string, which is what makes it
// assertable in a test with no terminal attached.
func (m FrameModel) View() string {
	return m.frame
}
