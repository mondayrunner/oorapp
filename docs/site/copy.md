# Oorapp, landing page copy

Language: English (see strategy.md). Site: oorapp.sitelane.nl. Every heading is one sentence. No em-dashes.

---

## 1. Hero

**Headline:** Every call written down, and the audio stays on your Mac.

**Subline:** Oorapp records your microphone and your computer's sound as two separate tracks, so it knows who said what in any call app. No bot joins the meeting. It is free and open source.

**Primary CTA:** Download for macOS
**Secondary CTA:** Read the code on GitHub
**Small print under the buttons:** macOS 14.4 or newer. Windows is next.

**Hero visual:** a real-looking transcript fragment from a made-up demo meeting, with speaker names in their colours, a waveform above it and one chapter heading. No abstract icon.

---

## 2. Two tracks

**Headline:** It hears both sides of the call, with nobody in the call noticing.

**Body:**
Most meeting tools send a bot into your call that announces itself. Oorapp stays on your Mac. It records what you say into your microphone and what your computer plays from the other side, each on its own track.

Because the two sides stay apart, every line in the transcript already knows whether it was you or them. When several people share one side, their voices are told apart too. Using speakers instead of a headset? The echo of the other side in your microphone is filtered out.

It works with Zoom, Google Meet, Microsoft Teams, Slack huddles and anything else that plays sound.

---

## 3. Local

**Headline:** Your meetings stay on your Mac, from the first word to the last.

**Body:**
Transcription runs on your own computer with Whisper, accelerated by your Mac's GPU. The audio and the text are never uploaded.

Every meeting is a plain folder in Documents/Meetings: the audio, a Markdown transcript and a small file that opens it in Oorapp. Delete the folder and the meeting is gone.

One thing is optional. If you have Claude Code or Codex installed, Oorapp can ask it to divide a long meeting into chapters. That sends the transcript text to the service you already use, with every tool of the agent switched off. Without an agent, nothing leaves your computer at all.

---

## 4. The transcript

**Headline:** You get a transcript you can listen to, fix and share.

**Body:**
Press stop and the lines appear as they are recognised. Give each speaker a name once and every line follows.

Click a line to hear it, drag through the waveform to find a moment, jump between chapters. Fix a word in place, give a line to the right speaker or delete it, with undo.

Copy the whole transcript with one click, or let your own scripts pick it up, for example to file it in Obsidian.

---

## 5. Notes

**Headline:** Hold one key anywhere, talk, and the text lands where you type.

**Body:**
For thoughts that are too short for a meeting. Hold the note key, say what you want to write and let go. A small pill at the bottom of the screen shows that Oorapp is listening, and the text is pasted into the app you were in.

A short tap keeps it listening until you tap again. Notes run on NVIDIA's Parakeet model, which turns a few sentences into text in well under a second.

---

## 6. History and your assistant

**Headline:** Everything you said is one search away, also for your AI assistant.

**Body:**
Meetings, notes and transcribed files share one history, grouped by day and searchable by word or date.

Drop a phone memo or an interview on the window to transcribe it, with the voices told apart.

With the Oorapp MCP server, Claude and other agents can search and read your meetings when you ask them to. "What did we agree on Tuesday?" gets an answer.

---

## 7. Closing CTA

**Headline:** Try it on your next call, it takes a minute to set up.

**Body:** Download Oorapp, allow the microphone and system audio, and press Start recording. The speech model downloads once on first use.

**Primary CTA:** Download for macOS
**Secondary CTA:** Read the code on GitHub

---

## 8. Credits

**Headline:** Oorapp is built on Jankees van Woezik's meeting recorder for Linux.

**Body:**
Jankees van Woezik made omarchy-meeting-recorder for Omarchy. The two-track recording, the way speakers are told apart, the echo filter, the transcript page and the test suite all come from his work.

Oorapp brings it to macOS and Windows, adds quick notes, a history and an MCP server, and keeps his MIT license. On Linux, use his: it is the original and it is excellent.

**Link:** github.com/jankeesvw/omarchy-meeting-recorder

---

## 9. FAQ

**Is it really free?**
Yes. There is no account and no subscription, and the code is MIT licensed. Tim van den Bosch made it at Sitelane because he wanted it himself.

**Which meeting apps does it work with?**
Any app that plays sound on your Mac: Zoom, Google Meet, Microsoft Teams, Slack, FaceTime or a browser tab. Oorapp records what your computer plays, so it does not need to know the app.

**Do I have to tell people I am recording?**
In many countries you do, and it is good manners everywhere. Oorapp does not announce itself, so telling the others is up to you.

**How good is the transcription?**
Meetings use Whisper large-v3-turbo on your Mac. It handles English, Dutch, German, French, Spanish, Italian and Portuguese, or detects the language itself. The app is tested against a set of recorded calls with people talking over each other, echo through speakers and music in the background.

**What does macOS ask permission for?**
The microphone, to hear you. System audio recording, to hear the other side. For notes it also asks for Input Monitoring, to hear the note key in other apps, and Accessibility, to paste the text there. On first use Oorapp downloads its speech models once: 1.6 GB for meetings and 670 MB for notes.

**Why not Granola, Otter or Fireflies?**
They are good products that work in the cloud, often with a bot or a subscription. Oorapp is for when you want the recording and the text on your own machine.

**Is there a Windows version?**
Windows is next. Follow the repository on GitHub to hear when it is ready.

---

## 10. Footer

Oorapp is free and open source under the MIT license. Made by Tim van den Bosch at Sitelane, based on omarchy-meeting-recorder by Jankees van Woezik. "Oor" is Dutch for ear.

Links: GitHub (github.com/mondayrunner/oorapp) · Original for Linux (github.com/jankeesvw/omarchy-meeting-recorder) · Sitelane (sitelane.nl)
