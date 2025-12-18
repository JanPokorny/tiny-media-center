# Tiny Media Center

Tiny Media Center (TMC) is a "10-foot UI" app intended for home media centers (i.e. HDMI TV boxes). Its defining characteristics include:

- Controllable using a "for Kodi" AliExpress remote -- meaning that main actions are done through keyboard interactions through arrow keys, enter, and back (KEY_BACK). A somewhat limited full keyboard is available on the back of the remote for typing but most interactions target the top side with these buttons.
- Filesystem-based. TMC assumes that the media are already properly named in given media folders, and it only uses the filename for display purposes. For storing metadata like watched percentage, TSC creates a `<filename stem>.tsc` file next to the media file.
- Simple. No metadata loading, no image previews, no fancy UI. It has the bare minimum features for a media center.
- Brutalist design. The UI uses a monospace font on black background, resembling a terminal window (but not a true terminal, allowing i.e. misaligned text in order to smoothly animate position). No graphical elements except text are used.

## Technology

Electron + React.

## Movie user flow

Mock-ups attached in ./ui/

- Main menu: user selects "movies"
- A list of movies shows
- User can scroll the list -- long lists have a scrolling behavior where the selected element is aligned in the middle of the screen and elements near top and bottom gradually turn to dark gray
- Optionally, user can start typing to fuzzy-filter the list
- User selects a movie
- Movie starts playing, no playback UI is shown
- Pressing Enter pauses the movie, showing a menu with options to unpause and some playback related options
- When playing, pressing an arrow left or right skips ahead or behind 10s, after six repeated presses in quick succession this briefly changes to one minute

## Settings

For now, use hardcoded settings (media folder path etc.), actual settings TBD.

## Other flows (shows, games, etc.)

TBD