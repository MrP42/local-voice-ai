# Recordings

Record meetings or import files, take notes live, transcribe them and condense everything into notes and minutes. Everything stays on this machine.

## How the page is laid out

- **Left: projects.** "All recordings", "No project" and your own projects, such as "Private" or "Client Stadtwerke". The "New project" icon creates one; double-click or F2 renames, Alt+Up/Down reorders, and the context menu (right click) offers the same. The meetings of the selected project are listed below; search and filters apply there only.
- **Middle: workspace.** The title (click to rename) with details ⓘ and menu ☰, date, duration, project and the participants (a click changes them), below them the tabs **Transcript** and **Minutes**.
- **Right: controls.** Start recording and Import file on top, then the icon row (Export, Follow-up mail, Copy, People) and the menu ☰. Below that are **Notes**, **AI notes** and **Questions**.
- Drag the handles between the columns with the mouse or adjust them with the arrow keys; double-click restores the default width, the arrow icons at the edge collapse a column. Widths, selection and tabs are kept for the next start. Every column scrolls on its own, the page never does.

## Organising projects and meetings

- Drag a meeting onto a project to **move** it. **Ctrl+drag** **adds** it there as well; it then lives in several projects.
- Without a mouse: menu ☰ or the project chip in the header, "Move to project …".
- Deleting a project never deletes a meeting; afterwards the meetings appear under "No project".

## Recording

- **Microphone** records you; **system audio** additionally captures what the computer plays, such as the other side of a video call. System audio is available on Windows; on the Mac recording uses the microphone only.
- Before the first recording the app asks for the participants' consent. Dictation is locked while recording.
- Take notes along: bullet points in the **Notes** tab are enough, each one remembers the recording time. The live transcript runs in the middle.
- If the app crashes, the recording is repaired on the next start and reappears in the list.

## Importing

Import audio and video files as well as subtitles (VTT, SRT) with the **Import file** icon in the controls, or drop them onto the workspace; several files at once work too. They land in the selected project and are transcribed like a recording.

### Queue

- You can add more files **at any time**, even while another one is still being transcribed. Each one gets its meeting at once, with the status **Waiting** and its **place** ("place 2 of 3", in the list and in the header). Files run in the order you added them.
- Move a waiting file **to the front** from the context menu (right mouse button) or the controls column, or **remove it from the queue**; it then counts as cancelled, and **Queue again** puts it at the end. A running file is stopped as before.
- The queue survives a restart. If a file was deleted or moved before its turn, the meeting reports it and the queue carries on with the next one.
- **A recording always takes priority:** while it runs, no new file starts and running imports pause at the next block. Afterwards the queue continues by itself.
- Settings, Dictation, Meetings has **Simultaneous transcriptions** (1, 2 or 3; default 1). Every additional transcription loads the model again and needs room: if memory (and, for GPU models, graphics memory) is short, the next file waits with the note "Waiting for memory". The machine is never driven to a standstill; when in doubt it stays at one.

## Notes, AI notes and minutes

- **Notes** are your text and stay yours. The **AI notes** are built from your bullet points and the transcript; every statement has a source, and a click on it jumps to the transcript.
- The **minutes** (summary, decisions, tasks) are written by the language model from the footer. A language model must be loaded or a provider connected.
- The **template** defines the sections. With "Automatic" the app picks a suitable one; in the menu ☰ you choose it with "Choose template …", manage templates with "Manage templates …" or regenerate notes and minutes with "Regenerate with template …".
- Transcription uses the model from the footer, or a dedicated one under Settings, Dictation, Meetings.

## Progress, pause and stop

While a meeting is processed, a progress bar shows the phase, percentage and time remaining. **Pause** halts processing and frees the computer, **Resume** continues. **Stop** cancels; the transcript so far stays, and you can continue later or re-transcribe.

## Menu ☰ and details

Rarely used actions live in the menu ☰: re-transcribe, regenerate AI notes and minutes, choose and manage templates, move to project, rename, details and delete. **Details** shows status, source, duration, consent, model, retention as well as the template and the file location of the minutes at a glance.

**Edit** in the details dialog changes the **title**, the **description** (multi-line), **date and time**, the **participants** (from the existing people) and the **projects**. Saving is all or nothing; file name and source stay as they are. The description is searchable and available to chat, AI notes, minutes and the local MCP server as background.

## Language and translation

- On import and on "Re-transcribe" the app detects the **language** of the recording and shows it as a chip in the header. A click shows where it came from and lets you correct it, optionally with an immediate re-transcription. Your own choice (model, fixed language) always wins.
- Menu ☰, **Translate to …**, creates the transcript as a **new version** in the target language. The **original stays untouched**; switch back with the version chip at any time. Timestamps and speakers are kept.
- Every translated sentence is checked for numbers, proper names and sentence count. Deviations are marked sentence by sentence next to the original in the **Compare** tab. The check does not catch errors of meaning, so look at marked sentences yourself.
- When regenerating minutes and AI notes you choose the **version** to base them on and the **language of the document**.

## Slides and image analysis

- For videos (screen recordings, presentations) the app looks for the **slides** after the import and stores them as thumbnails in the **Slides** tab. "Detect slides" in the menu ☰ does it later. A click opens the large view or plays from that point; single slides can be hidden.
- Slide text is read with Windows text recognition and flows into the minutes, with its source. Please check numbers, section signs and tables against the image.
- Optional, under Settings, Dictation, Meetings: **Image analysis for slides**. A local vision model reads the slides more precisely and describes each in one sentence. It needs a graphics card (at least 6 GB free) and the image projector (about 1 GB, downloaded only on request) and is unloaded after the job. Without it, Windows text recognition stays in use. The app also needs ffmpeg on the path.

## Minutes across several recordings

In a project: menu ☰ or right click on the project, **Minutes together**. Tick at least two recordings **with a transcript**, pick a template and **Minutes** (detailed) or **Summary** (short). The result appears as "Project minutes" in the project; every statement names the recording and time, a click jumps there. **Stop** is available during the run; if one recording has no transcript nothing is written halfway.

## Small window

When the window gets narrow, for example next to a video call, the projects fold into a drawer ("Open projects") and workspace (transcript, minutes) and notes share the height. Adjust the divider between them with the mouse or Up/Down arrows. Recording, taking notes and reading the live transcript all work this way, without changing pages.

## Retention

By default the audio is deleted once the minutes exist. Transcript and minutes stay. Adjustable under Settings, General.
