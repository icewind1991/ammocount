# Ammocount

Extract various information from tf2 demo files for usage in frag movies.

## Usage

```
ammocount.exe <demo file> <player name or steamid> <start tick> <end tick>
```

## Output

The output consists of multiple text files placed next to the demo file containing bits of information per output frame (at 120fps) that are intended to be imported into after effects code.

This output format is created for a specific AE workflow and probably not optimal. But it should be fairly easy to adapt for other uses. 