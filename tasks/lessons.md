# Lessons

## Window top row

- The window controls float over the panes and fill the sidebar's 236pt up to the reload
  button. Never append to that row: anything wider than a badge lands on the pane beside it.
  Status goes inside the pane's own header.
- Everything in the top row is centred on y = 25 (traffic lights, window controls, list
  title, reading toolbar). A new header element goes on that line.
- A bar is `ui::TOP_BAR` tall with its content centred. Never reach that line with uneven
  top and bottom padding: the gaps show as soon as a button has a background.
- Check a header change with the sidebar and the list each open and closed, an empty
  mailbox, loading, both themes, and the list at its 300pt minimum.

## Reading mail

- Anything that changes how the sender's mail looks gets a switch in Settings › Reading, on
  by default when it is the better experience. Do not plan such a feature without one.
- The CSP is not the whole lockdown: meta refresh and preconnect both get past it. Test a
  web view change with a hostile mail and a localhost listener, and with a negative control.
