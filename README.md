# hexzen

a simple, blazingly fast hex editor

## Usage

```
Usage: hexzen [OPTIONS] <FILE>

Arguments:
  <FILE>

Options:
  -d, --dump     prints a hex dump instead of opening the editor
  -u             use the unicode replacement character instead of a dot when a character isn't printable ascii
  -h, --help     Print help
  -V, --version  Print version
```

## Keybinds

* `←`, `↑`, `→`, `↓`: move the cursor
* `PgUp`, `PgDown`: scroll up or down
* `Tab`: toggle between normal and text modes
* `Esc`: set the editor into normal mode

### Normal mode

* `u`, `z`: undo
* `r`: redo
* `w`: save changes
* `q`: exit the program without saving
* `j`: jump to an arbitrary position in the file
