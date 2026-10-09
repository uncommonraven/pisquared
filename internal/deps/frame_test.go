package deps

import (
	"bytes"
	"context"
	"strings"
	"testing"
	"time"

	tea "github.com/charmbracelet/bubbletea"
)

// A compile-time assertion that the model is one bubbletea can run. It fails to
// build rather than failing a test if the interface moves, which is the earlier
// and clearer of the two failures.
var _ tea.Model = FrameModel{}

// key returns the message a terminal would send for a typed key. Constructing it
// directly is what lets Update be exercised with no terminal attached.
func key(r rune) tea.KeyMsg {
	return tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{r}}
}

func TestFrameModelRendersTheFrame(t *testing.T) {
	const frame = "pisquared\n\n  choosing a device"

	if got := NewFrameModel(frame).View(); got != frame {
		t.Errorf("View() = %q, want %q", got, frame)
	}
}

func TestFrameModelQuitsOnQ(t *testing.T) {
	for _, tc := range []struct {
		name string
		msg  tea.Msg
	}{
		{"q", key('q')},
		{"ctrl+c", tea.KeyMsg{Type: tea.KeyCtrlC}},
	} {
		t.Run(tc.name, func(t *testing.T) {
			model, cmd := NewFrameModel("frame").Update(tc.msg)
			if cmd == nil {
				t.Fatalf("Update(%s) returned no command; expected it to quit", tc.name)
			}
			// tea.Quit is a function, so it cannot be compared. Running it and
			// naming the message it produces is how the instruction is checked.
			if _, ok := cmd().(tea.QuitMsg); !ok {
				t.Errorf("Update(%s) returned a command that did not quit", tc.name)
			}
			if _, ok := model.(FrameModel); !ok {
				t.Errorf("Update(%s) returned a %T; expected it to keep the model", tc.name, model)
			}
		})
	}
}

func TestFrameModelIgnoresEveryOtherKey(t *testing.T) {
	const frame = "frame"
	model, cmd := NewFrameModel(frame).Update(key('a'))

	if cmd != nil {
		t.Errorf("Update(a) returned a command; a key that is not q should change nothing")
	}
	if got := model.View(); got != frame {
		t.Errorf("Update(a) changed the frame to %q, want %q", got, frame)
	}
}

// TestFrameModelRendersAFrameAndQuitsWithNoTerminalAttached runs the model
// through a whole bubbletea program with its input and output supplied, which
// is the part of the dependency this repository actually depends on: that the
// program renders and exits without a terminal being involved.
func TestFrameModelRendersAFrameAndQuitsWithNoTerminalAttached(t *testing.T) {
	const frame = "pisquared\n\n  quitting on q"

	// The context is a backstop rather than the mechanism. If q ever stopped
	// quitting, the run would otherwise hang until the test binary's own
	// timeout, and the failure would say so less usefully.
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()

	var out bytes.Buffer
	program := tea.NewProgram(
		NewFrameModel(frame),
		tea.WithContext(ctx),
		tea.WithInput(strings.NewReader("q")),
		tea.WithOutput(&out),
		tea.WithoutSignalHandler(),
	)

	if _, err := program.Run(); err != nil {
		t.Fatalf("Run() = %v, want no error", err)
	}

	// bubbletea's renderer terminates each line with CRLF, because it is
	// writing for a terminal that may be attached to nothing. Normalising that
	// back lets the frame be compared as the string it is.
	rendered := strings.ReplaceAll(out.String(), "\r\n", "\n")
	if !strings.Contains(rendered, frame) {
		t.Errorf("the program wrote %q, which does not contain the frame %q", rendered, frame)
	}
}
