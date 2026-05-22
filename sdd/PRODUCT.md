# Product Definition

## What is Rologlyphex?

Rologlyphex is a desktop companion for programmable macropads. It turns a simple knob-and-buttons device into a rotating deck of Unicode character palettes -- arrows, math symbols, emoji -- with a visual heads-up display that shows what each button will type at any moment.

The name comes from Rolodex (the rotating card file) and glyphs (written symbols). Spin the knob, see the deck, press a button, get the character.

## What problem does it solve?

Typing special characters on a computer is tedious. Character pickers require multiple clicks and interrupt your workflow. Memorizing keyboard shortcuts is impractical for more than a handful of symbols. Copy-pasting from reference sheets breaks your flow.

A dedicated macropad with three buttons can type three characters instantly -- but only three. That's not enough. To be useful, the same three buttons need to mean different things at different times, and the user needs to know what they mean *right now*.

Rologlyphex solves both problems:

- **Deck cycling**: A rotary knob switches between named layouts (Arrows, Symbols, Emoji, etc.), each assigning different characters to the same physical buttons. One small device, unlimited character sets.
- **Visual feedback**: A floating overlay appears briefly in the corner of the screen whenever the deck changes, showing the layout name and what each button will type. No memorization required.
- **Instant input**: Pressing a button types the character directly into whatever application has focus -- text editors, chat apps, terminals, IDEs. No clipboard, no popups, no interruption.

## Who is it for?

- Developers who frequently type Unicode operators, arrows, or mathematical symbols in code and documentation
- Writers and communicators who use emoji or special punctuation in messaging and notes
- Anyone with a programmable macropad who wants more from three buttons and a knob

## Why does it exist?

Standard Linux tools can map macropad buttons to characters, but they fail at the "rotating deck" experience:

- **keyd** can switch layouts, but provides no visual indication of which layout is active or what the buttons do
- **xdotool** can type characters, but is unreliable when invoked from system services and adds noticeable latency
- **Character pickers** (GNOME Characters, emoji selectors) require mouse interaction and take focus away from your work

Rologlyphex fills the gap: it is the visual layer and input engine that makes a rotating character deck actually usable. It watches for layout changes, shows what's active, and types characters reliably -- all without stealing focus or interrupting the user's workflow.

## How is it used?

1. Plug in a programmable macropad
2. Start Rologlyphex (runs as a background service)
3. Spin the knob to browse character decks -- a brief overlay shows each deck's name and characters
4. Press a button to type the corresponding character into whatever you're working on
5. The overlay disappears on its own after a few seconds

Adding new character decks requires only editing a configuration file. No firmware changes, no recompilation, no restart.
