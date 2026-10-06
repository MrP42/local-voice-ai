# Read aloud

Put text in the middle, pick a voice, press Read. Everything runs on this machine; nothing leaves it.

## What this page does

- **Text**: type, paste or dictate it (microphone icon).
- **Icon row** below the voice picker: equally sized icons without labels. A tooltip gives name and effect, on mouse hover after a short pause and on keyboard focus. Rarely used actions sit behind the menu (☰) at the right edge.
- **Add (+)**: bring in a document (TXT, MD, PDF, DOCX), a web address or a file for the project.
- **Translate** (language icon, on the Translation tab, target language next to it) and **Summarize** (document icon, on the Summary tab; length, detail and audience are behind the sliders icon next to it) put their result on a separate tab. The original stays untouched.
- **Read** speaks sentence by sentence. The arrows jump to the previous or next sentence, Pause holds.
- **Save as audio** (arrow down) writes the recording into the page's project folder. It shows up under Files on the right.
- **Pre-generate changes** (bolt) puts changed sentences into the cache ahead of time without playing them.
- **Menu (☰)**: script workshop, clean up text, check script and auto-tagging. Script errors show as a red number on the menu icon.

## Pages (left)

Each page is a worksheet with its own text and folder. The list shows the start of the text and when it last changed. Double-click renames.

<!--if:fish-->
## Speakers and emphasis in the text

- **Speaker change**: start a line with a voice name and a colon, for example `Olga:`. Everything up to the next change is spoken by that voice.
- **Style**: `<Olga:whispering>` picks a saved style of that voice.
- **Tags** go in square brackets exactly where they should act: `[whisper] Come closer.` or `He opened the door. [short pause] Nothing.`
- **Auto-tagging** (menu ☰) suggests tags via the language model. It only inserts, never deletes. Accept suggestions one by one or undo them.
- The palette below the text lists all tags by group. Click inserts at the cursor.

<!--/if:fish-->

## Voices

- Select in the bar above the player. Piper voices show language and quality there, for example "Thorsten · German · HQ · Piper". Each tab remembers its voice.
<!--if:fish-->
- Listen, clone, import and delete: **Settings → Read aloud**, or via "Manage voices …" at the end of the voice list.
<!--/if:fish-->
<!--if:fish-->
- **Cloning** needs a 10 to 30 second reference. The transcript is generated and can be corrected.
- The **seed** shapes the default voice. A seed you like can be saved as a named voice.
- Speaker changes in the text work with Fish Speech voices. Piper reads everything in the selected voice.
<!--/if:fish-->
<!--if:nofish-->
- Piper reads the whole text in the selected voice. Speaker changes, cloning and styles belong to **Fish Speech**, an optional extra engine for the graphics card. It is not set up on this computer; enter its folder under **Settings → Read aloud**.
<!--/if:nofish-->

<!--if:fish-->
## Two engines

| | Fish Speech | Piper |
|---|---|---|
| Runs on | GPU (NVIDIA, 6 GB VRAM or more) | CPU |
| Voices | cloned, seed, styles, tags | fixed catalog voices |
| Start | server, 20 to 90 s | instant |
| Quality | natural, expressive | clear, even |

The engine is chosen under **Settings → Read aloud**. Piper voices are downloaded on the Models page under Reading Voices.
<!--/if:fish-->
<!--if:nofish-->
## Setting up speech output

Piper runs on the CPU, starts instantly and needs only a small voice. Download it under **Models → Reading voices**. If the Piper program is incomplete, the voice shows as "not usable" there and "Install program" repairs it.
<!--/if:nofish-->

## Language model and server in the footer

- **Language model** (footer, drop-up menu): the model for translating, summarizing and auto-tagging. Its light pulses yellow while it works; the menu offers “Warm up” (loads it for ten minutes) and “Unload” (frees the memory).
<!--if:fish-->
- **Server** (footer, left of the shield): the Fish Speech server. Grey off, yellow starting, green running, orange error. A click asks first: start, restart or stop.
<!--/if:fish-->

## When something is stuck

<!--if:fish-->
- **Start takes long**: close other GPU programs, the server needs free video memory.
<!--/if:fish-->
- **Text gets cut**: the limit lives under Settings → Read aloud, maximum characters per job.
<!--if:fish-->
- **Blank page or error in the header**: stop the server and start it again. If it persists, check the Fish Speech folder in Settings.
<!--/if:fish-->
<!--if:nofish-->
- **Voice "not set up"**: download or repair it under **Models → Reading voices**; it then appears in the list right away.
<!--/if:nofish-->

