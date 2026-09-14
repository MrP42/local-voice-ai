# Read aloud

Put text in the middle, pick a voice, press Read. Everything runs on this machine; nothing leaves it.

## What this page does

- **Text**: type, paste or dictate it (microphone button).
- **Add (+)**: bring in a document (TXT, MD, PDF, DOCX), a web address or a file for the project.
- **Translate** and **Summarize** put their result on a separate tab. The original stays untouched.
- **Read** speaks sentence by sentence. The arrows jump to the previous or next sentence, Pause holds.
- **Save audio** writes the recording into the page's project folder. It shows up under Files on the right.

## Pages (left)

Each page is a worksheet with its own text and folder. The list shows the start of the text and when it last changed. Double-click renames.

## Speakers and emphasis in the text

- **Speaker change**: start a line with a voice name and a colon, for example `Olga:`. Everything up to the next change is spoken by that voice.
- **Style**: `<Olga:whispering>` picks a saved style of that voice.
- **Tags** go in square brackets exactly where they should act: `[whisper] Come closer.` or `He opened the door. [short pause] Nothing.`
- **Auto-tagging** suggests tags via the language model. It only inserts, never deletes. Accept suggestions one by one or undo them.
- The palette below the text lists all tags by group. Click inserts at the cursor.

## Voices

- Select in the bar above the player. Piper voices show language and quality there, for example "Thorsten · German · HQ · Piper". Each tab remembers its voice.
- Listen, clone, import and delete: **Settings → Read aloud**, or via "Manage voices …" at the end of the voice list.
- **Cloning** needs a 10 to 30 second reference. The transcript is generated and can be corrected.
- The **seed** shapes the default voice. A seed you like can be saved as a named voice.
- Speaker changes in the text work with Fish Speech voices. Piper reads everything in the selected voice.

## Two engines

| | Fish Speech | Piper |
|---|---|---|
| Runs on | GPU (NVIDIA, 6 GB VRAM or more) | CPU |
| Voices | cloned, seed, styles, tags | fixed catalog voices |
| Start | server, 20 to 90 s | instant |
| Quality | natural, expressive | clear, even |

The engine is chosen under **Settings → Read aloud**. Piper voices are downloaded on the Models page under Reading Voices.

## Icons in the page header

- **Brain**: the language model for translating, summarizing and auto-tagging. Click preloads or unloads it.
- **Server**: the Fish Speech server. Grey off, yellow starting, green running, orange error. Click does what this state calls for.

## When something is stuck

- **Start takes long**: close other GPU programs, the server needs free video memory.
- **Text gets cut**: the limit lives under Settings → Read aloud, maximum characters per job.
- **Blank page or error in the header**: stop the server and start it again. If it persists, check the Fish Speech folder in Settings.
