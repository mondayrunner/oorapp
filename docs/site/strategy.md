# Oorapp, strategie en positionering

Korte strategiesessie (sitelane-strategy-session, lean). Oorapp is gratis en open source, dus prijsopties vallen weg: de "prijs" die iemand betaalt is vertrouwen en installatiemoeite.

## Positionering in het kort

Oorapp is een meeting recorder voor de Mac die beide kanten van elk gesprek vastlegt zonder bot in de call, alles op je eigen computer uitschrijft en gratis en open source is. Dezelfde app zet een ingesproken gedachte om in tekst op de plek waar je typt.

De ene belofte: **elk gesprek op papier, en de audio blijft op je eigen Mac.**

## Doelgroep

- Primair: mensen die veel videocalls doen op een Mac en een verslag willen zonder dat er een bot meekijkt. Freelancers, consultants, coaches, kleine teams, product- en tech-mensen. Ze kennen Granola, Otter of Fireflies, en voelen ongemak bij een bot die zichzelf aankondigt of bij gesprekken die op andermans server staan.
- Secundair: mensen die nu dicteren met Paraspeech, Superwhisper of Wispr Flow, en dat graag in dezelfde app willen als hun meetings.
- Tertiair: developers en AI-gebruikers die hun meetings aan Claude of een andere agent willen geven (MCP), en die open source zelf willen lezen.

## Fase 1, samengevat

**Product Vision Canvas**
- Target group: Mac-gebruikers met veel calls, privacybewust, comfortabel met tools.
- Needs: een betrouwbaar verslag met wie wat zei; geen bot die het gesprek ongemakkelijk maakt; geen gevoelige gesprekken in de cloud; snel een gedachte vastleggen zonder te typen; later iets terugvinden.
- Product: opname in twee sporen (jouw microfoon en het computergeluid), lokale transcriptie (whisper large-v3-turbo op Metal, Parakeet voor notities), sprekers uit elkaar, hoofdstukken via je eigen coding agent, speler gekoppeld aan het transcript, notities met een sneltoets die direct plakken, bestanden uitschrijven, doorzoekbare geschiedenis, MCP-server.
- Business goals: gratis weggeven, gebruikers en bijdragers op GitHub, Sitelane laten zien als maker van goede tools, krediet aan de upstream maker.
- Vision: je gesprekken zijn van jou, en worden gewoon tekst waar je mee verder kunt.

**User stories (selectie)**
1. Als consultant wil ik een klantgesprek opnemen zonder bot, zodat de klant zich niet bekeken voelt. Gegeven een Zoom-call, als ik op Start druk, dan neemt Oorapp mijn microfoon en het geluid van de call op zonder dat iemand in de call iets ziet.
2. Als gebruiker wil ik zien wie wat zei, zodat ik het verslag niet hoef te ontrafelen. Gegeven een opname, als de transcriptie klaar is, dan staat elke regel bij een spreker die ik een naam kan geven.
3. Als privacybewuste gebruiker wil ik dat niets mijn computer verlaat, zodat ik vertrouwelijke gesprekken kan opnemen. Gegeven een opname, als hij wordt uitgeschreven, dan gebeurt dat op mijn Mac zonder upload.
4. Als drukke maker wil ik een gedachte inspreken en de tekst direct in mijn editor hebben, zodat ik niet hoef te typen. Gegeven elke app, als ik de notitietoets vasthoud en praat, dan staat de tekst na loslaten waar mijn cursor stond.
5. Als gebruiker wil ik een spraakmemo van mijn telefoon laten uitschrijven, zodat ook die in mijn archief staat.
6. Als gebruiker wil ik een gesprek van vorige week terugvinden op een woord, zodat ik een afspraak kan opzoeken.
7. Als Claude-gebruiker wil ik dat mijn agent mijn meetings kan lezen, zodat ik kan vragen "wat spraken we dinsdag af".
8. Als gebruiker wil ik hoofdstukken in een lang gesprek, zodat ik snel naar het juiste stuk spring.
9. Als gebruiker wil ik een regel corrigeren of aan de juiste spreker geven, zodat het verslag klopt.
10. Als skepticus wil ik de broncode kunnen lezen, zodat ik weet wat de app met mijn audio doet.

**Messaging Canvas**
- Struggle: aantekeningen maken en tegelijk luisteren lukt niet; meeting-bots zijn ongemakkelijk en soms niet toegestaan; cloud-tools vragen een abonnement en je gesprekken; dicteren en meetings zitten in twee apps.
- Solution: Oorapp luistert naar wat je zegt en wat je computer afspeelt, en schrijft alles lokaal uit met sprekers.
- Hesitations: is lokale transcriptie goed genoeg; is het lastig te installeren (toestemmingen, model van 1,6 GB); is gratis te mooi om waar te zijn; werkt het met mijn meeting-app; moet ik iedereen vertellen dat ik opneem.
- Awareness: probleembewust en oplossingsbewust. Ze kennen de categorie, ze zoeken een variant zonder bot en zonder cloud.
- Differentiators: geen bot, werkt met elke meeting-app, alles lokaal, gratis, open source, meetings en dicteren in één app, je eigen agent via MCP.
- Success: na elk gesprek een leesbaar verslag met namen, zonder nadenken; een gedachte binnen seconden als tekst; alles op één plek doorzoekbaar.

## Panel

**Jonathan Stark (propositie).** Er is geen prijs, dus de waarde moet ergens anders tastbaar worden. Laat zien wat niets doen kost: een gemist besluit, een uur uitschrijven, een maandabonnement op een cloud-tool. Hij wil een vergelijking met betaalde alternatieven op de pagina, zodat "gratis" niet als hobbyproject leest. Zijn rode vlag: "lokaal en privé" claimt iedereen tegenwoordig. Maak het concreet: geen bot, de bestanden staan als gewone map in Documents.

**Alex Napier Holland (copy en opbouw).** Eens met concreet maken, oneens met de vergelijkingstabel: die zet Oorapp in de taal van de concurrent. De headline-test: "AI meeting notes" kan op elke site staan, "zonder bot in de call" niet. Opbouw: hero, hoe het luistert (twee sporen), privacy, het transcript, notities, geschiedenis en je agent, krediet, FAQ. Elke feature naar een voordeel: twee sporen wordt "hij weet wie wat zei", lokale whisper wordt "je gesprek gaat nergens heen".

**Alex M H Smith (strategie).** Twee sporen is het echte "only": de app hoort jouw kant en de andere kant apart, daardoor kan hij zonder bot en zonder stemherkenning toch zeggen wie wat zei. Bouw daarop, niet op "beter dan Granola". Hij neemt Stark's tabel weg: wie zich vergelijkt met de marktleider, wordt "de gratis versie van". De notities zijn sterk, maar mogen de belofte niet verdunnen; ze komen tweede. De Toilet Paper Rule: niemand wil over diarisatie lezen, dus de techniek hoort in de FAQ.

**Rory Sutherland (gedrag).** De grootste blokkade is sociaal: de bot die "Otter is recording" in de chat zet maakt het gesprek ongemakkelijk. Dat is de emotionele kern, sterker dan privacy in abstracte zin. De DoubleTree-cookie: het transcript is klaar met namen en hoofdstukken terwijl je je koffie haalt, en de bestanden zijn gewone bestanden in een map. Het krediet aan Jankees werkt als Prattfall: openlijk zeggen dat het op andermans werk is gebouwd maakt het geloofwaardiger. Zijn nudge: toon een echt stukje transcript met sprekersnamen in de hero, niet een abstract icoon. Oneens met Alex NH over de taal: een Nederlandse site past bij de naam en bij Tim's netwerk.

**David Ogilvy (big idea).** Het idee is "beide kanten van het gesprek, zonder dat iemand het merkt", en dat houdt dertig jaar. Eén belofte op de hero, de rest volgt. Specifics verkopen: macOS 14.4, Zoom, Meet en Teams bij naam, model van 1,6 GB eenmalig, MIT-licentie. Hij is het oneens met Rory over de taal: de lezers die open source verspreiden, op GitHub en Hacker News, lezen Engels; de naam blijft Nederlands en krijgt één zin uitleg. Hij wil "gratis" niet verstoppen: zet het in de subline, eerlijk en zonder uitroepteken.

## Synthese

**Consensus.** Eén belofte: elk gesprek op papier, zonder bot en zonder dat de audio je Mac verlaat. Twee sporen is het verhaal achter "wie zei wat". Krediet aan Jankees prominent en warm. Techniek naar de FAQ. Een echt transcriptfragment in de hero.

**Conflicten.**
- Vergelijkingstabel (Stark) tegen geen vergelijking (Alex NH, Smith). Besluit: geen tabel. Eén zin in de FAQ beantwoordt "waarom niet Granola of Otter", zonder namen af te kraken.
- Taal: Nederlands (Rory) tegen Engels (Ogilvy, Alex NH). **Besluit: Engels.** Open source groeit via GitHub, Hacker News en de Omarchy-community van Jankees, en de app zelf is Engelstalig. De Nederlandse naam wordt een sympathiek detail ("oor" is Dutch for ear). Een Nederlandse versie kan later als tweede pagina.
- Notities naast meetings: Smith wil ze ondergeschikt, Rory ziet ze als dagelijkse gewoonte die de app op het dock houdt. Besluit: tweede sectie na het transcript, met een eigen kop, niet in de hero.

**Geïntegreerd advies.** Positioneer Oorapp als de meeting recorder die beide kanten van het gesprek hoort zonder bot, gratis en op je eigen Mac. Laat het transcript zelf het bewijs zijn. Bouw vertrouwen met openheid: de code, de licentie, de herkomst.

**Drie eerste acties.**
1. Neem een korte, verzonnen demo-meeting op en gebruik een echt fragment met namen als hero-beeld.
2. Ontwerp de pagina in Paper op basis van copy.md; review per sectie met Mobbin.
3. Zet de repo publiek op github.com/mondayrunner/oorapp met een README die dezelfde belofte en hetzelfde krediet gebruikt.
