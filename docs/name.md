# The name

Dupin is named after C. Auguste Dupin, Poe's detective and the source of the line of Mac launchers that runs Sherlock → Watson → Alfred. Apple named its search tool after Sherlock Holmes; Karelia's Watson took the name of Holmes's companion; Alfred took a butler's. Dupin came before all of them: Holmes's author owed the detective story to Poe. This page tells that story, and what Dupin takes from the launchers that came before it.

## C. Auguste Dupin

Edgar Allan Poe wrote three stories about C. Auguste Dupin, an amateur who solves crimes in Paris by reasoning, told by an unnamed friend who shares his rooms:

- "The Murders in the Rue Morgue", in _Graham's Magazine_, April 1841.
- "The Mystery of Marie Rogêt", in three parts in _Snowden's Ladies' Companion_, November and December 1842 and February 1843.
- "The Purloined Letter", in _The Gift_ for 1845, published in 1844.

Dupin is generally acknowledged as the first detective in fiction. E. T. A. Hoffmann's _Das Fräulein von Scuderi_ (1819) is sometimes cited as earlier.

Arthur Conan Doyle acknowledged the debt. In _A Study in Scarlet_ (1887), the first Holmes story, Watson tells Holmes he reminds him of Poe's Dupin, and Holmes answers that Dupin "was a very inferior fellow". Doyle didn't share his detective's opinion. Presiding at the Poe centenary dinner in London on 1 March 1909, he asked: "Where was the detective story until Poe breathed the breath of life into it?"

## Sherlock

Apple named its search tool for the Mac after Holmes. Sherlock shipped with Mac OS 8.5 in 1998, an extension of the Finder's own search that looked for files and searched the web from one window. Sherlock 2 came with Mac OS 9 in 1999 and introduced channels.

Sherlock 3 came with Mac OS X 10.2 Jaguar, released on 24 August 2002, and turned the channels into small applications built on web services: Pictures, Stocks, Movies, Yellow Pages, eBay, Flights, Dictionary, Translation and AppleCare among them.

Mac OS X 10.4 Tiger (April 2005) moved searching for files to Spotlight. Sherlock still shipped with Tiger, though most of Apple's own channels had stopped working, and Mac OS X 10.5 Leopard (2007) left it out.

## Watson and "Sherlocked"

Karelia Software's Watson, written by Dan Wood and released in November 2001, took Sherlock 2's channels further in a Mac OS X app: each of its tools fetched one kind of information from a web service and showed it in its own native view. It won the 2002 Apple Design Award for the most innovative Mac OS X product.

Months later Sherlock 3 did much the same. Ars Technica's Jaguar review called it "a dead-ringer for Watson", though the two had no formal relationship; Apple's view was that Sherlock 3 was the natural next step from Sherlock 2. In 2004 Watson's technology went to Sun Microsystems (Karelia says Sun bought it; Wikipedia says Sun licensed it), and Karelia stopped developing the Mac app and made it free.

The episode gave Mac developers a word. When Apple builds into its own software what a third-party app already does, the app has been _Sherlocked_.

## The launchers

Sherlock and Watson searched; launchers find the thing you want on your own Mac and act on it from the keyboard. Their line runs alongside.

### LaunchBar

LaunchBar, by Objective Development, began on NeXTSTEP: the idea dates from 1995 and the first public version from 1996. Its authors say they chose Command-Space as its hotkey in 1995. It came to Mac OS X in 2001 as LaunchBar 3. You type a short abbreviation, and LaunchBar learns which item you mean by it: the abbreviation you used to pick something becomes a stronger match for it next time.

### Quicksilver

Quicksilver, written by Nicholas Jitkoff at Blacktree from 2003, made a grammar of it. Its window has three panes: an object, an action and an optional second object, like the subject, verb and object of a sentence. You pick a file, then an action such as sending it by email, then whom to send it to. Triggers bind such a command to a hotkey, and plug-ins add objects and actions. Jitkoff opened its source in 2007, under the Apache License 2.0, and a community has maintained it since.

### Butler and Quick Search Box

Peter Maurer's Butler, first called Another Launcher, was a launcher of the same era, by 2004; it is still sold by Many Tricks. Jitkoff moved to Google, where he and Dave MacLachlan released Quick Search Box for the Mac in January 2009, an open-source launcher in Quicksilver's manner. Its last build is from 2010, and it was left unfinished.

### Alfred

Alfred, by Andrew and Vero Pepperrell's Running with Crayons, came out in February 2010, a launcher with a butler's name. Like LaunchBar it learns from what you pick: it ranks what you use often higher, and latches the exact query you typed to the result you chose, so that query shows it first. The paid Powerpack adds clipboard history, snippets and, since Alfred 2 in 2013, workflows: chains of inputs, actions and outputs that users build and share.

### Spotlight

Spotlight came with Mac OS X 10.4 Tiger in 2005 as file search in the menu bar, and grew into a launcher: Command-Space opens it today. OS X 10.10 Yosemite (2014) moved it to a window in the middle of the screen with results from Wikipedia, Maps and the web. macOS 26 Tahoe (2025) gave it actions from apps, quick keys and clipboard history, features that third-party launchers had offered for years.

### Raycast

Raycast, founded in 2020 by Thomas Paul Mann and Petr Nikolaev, opened a public beta that October. Its extensions are written in TypeScript with React and run on Node; since November 2021 they have been published through the Raycast Store, whose extensions live in one public GitHub repository.

## What Dupin takes from each

Dupin is a launcher in this line, and borrows from it openly:

- **Learning from what you pick.** As LaunchBar and Alfred do, Dupin remembers which result you opened for which query, and when, so the same query puts it first next time, and a shorter query that begins a longer one you used ranks it higher too. A pick counts less as it ages, as a visit does, so an old one gives way to what you pick now. Beside that it keeps frecency, how often you opened something weighted towards recently, with the age buckets Firefox's address bar uses. Both live in [`history.rs`](../crates/dupin-core/src/history.rs), and the boost is bounded so that a strong match still beats a weak one you open often.
- **Kinds and actions.** Quicksilver's object and action survive as Dupin's kinds: every row is text, a URL, a file or an app, and its kind gives it actions. Cmd+K opens a file's [actions](./features/file-actions.md), such as Open with…, Move to… and Send to…, and [`[advanced]`](./features/advanced.md#kind-actions) adds your own actions to every row of a kind, run by a script, a URL or an extension.
- **Extensions in TypeScript, without a store.** As Raycast's are, Dupin's [extensions](./features/extensions.md) are TypeScript (or JavaScript) packages, and Dupin draws their views. They run in QuickJS-ng in a sandboxed helper process, not Node, and reach nothing on their own: each declares the capabilities it wants, such as one website or a folder you pick, and you allow each one, and can revoke it ([Asking for more](./features/extensions.md#asking-for-more)). There is no store or review: packages are installed from Git and pinned by commit, and `dupin ext search` lists GitHub repositories with the `dupin-extension` topic, marked unreviewed, because the broker, not a review, protects you ([design](./design/extension-platform.md#packaging-and-distribution)).
- **Your data stays yours.** There is no account. What Dupin keeps for features such as clipboard history, snippets and extensions' storage is [encrypted on disk](./features/storage.md) under a key kept in your Keychain. Syncing between Macs is [designed](./design/sync.md) the same way: end-to-end encrypted, through a folder you already sync, such as iCloud Drive, with no Dupin server. It is not built yet.
- **Keywords and aliases.** As in Alfred, a short [keyword](./features/keywords.md) and Space enters a command (`cal`, `clip`, `define`), and `g` or `gh` starts a [web search](./features/web-searches.md). [`[aliases]`](./features/keywords.md#aliases) lets you add your own shorthands for a command, an app, a web search or a file.

## Sources

- Wikipedia: [C. Auguste Dupin](https://en.wikipedia.org/wiki/C._Auguste_Dupin), [The Murders in the Rue Morgue](https://en.wikipedia.org/wiki/The_Murders_in_the_Rue_Morgue), [The Mystery of Marie Rogêt](https://en.wikipedia.org/wiki/The_Mystery_of_Marie_Rog%C3%AAt), [The Purloined Letter](https://en.wikipedia.org/wiki/The_Purloined_Letter)
- The Edgar Allan Poe Society of Baltimore: [Rue Morgue](https://www.eapoe.org/works/info/pt030.htm), [Marie Rogêt](https://www.eapoe.org/works/info/pt040.htm), [The Purloined Letter](https://www.eapoe.org/works/info/pt051.htm)
- Arthur Conan Doyle, _A Study in Scarlet_, chapter 2, [Project Gutenberg #244](https://www.gutenberg.org/ebooks/244)
- Arthur Conan Doyle Encyclopedia: [Lecture in memory of Edgar Allan Poe](https://www.arthur-conan-doyle.com/index.php/Lecture_in_memory_of_Edgar_Allan_Poe) (1 March 1909)
- Wikipedia: [Sherlock (software)](<https://en.wikipedia.org/wiki/Sherlock_(software)>), [Mac OS 9](https://en.wikipedia.org/wiki/Mac_OS_9), [Karelia Watson](https://en.wikipedia.org/wiki/Karelia_Watson)
- Apple: [Apple Introduces Jaguar](https://www.apple.com/newsroom/2002/07/17Apple-Introduces-Jaguar-the-Next-Major-Release-of-Mac-OS-X/) (17 July 2002), [Sherlock 3 in Jaguar](https://web.archive.org/web/2002id_/http://www.apple.com/macosx/jaguar/sherlock.html) (archived)
- Ars Technica: [Mac OS X 10.2 review, Sherlock 3](https://archive.arstechnica.com/reviews/02q3/macosx-10.2/m-macosx-10.2-13.html)
- TidBITS: [Apple Names 2002 Design Award Winners](https://tidbits.com/2002/05/13/apple-names-2002-design-award-winners/)
- Karelia Software: [company history](http://staging.karelia.com/company/press/karelia-company-history.pdf) (PDF)
- O'Reilly: [interview with Norbert Heger of Objective Development](https://web.archive.org/web/2008/http://www.oreillynet.com/pub/a/mac/developer/2003/09/25/innovators.html) (2003, archived); Wikipedia: [LaunchBar](https://en.wikipedia.org/wiki/LaunchBar); Objective Development: [LaunchBar](https://www.obdev.at/products/launchbar/index.html)
- Wikipedia: [Quicksilver](<https://en.wikipedia.org/wiki/Quicksilver_(software)>); TUAW: [Quicksilver goes open source](https://web.archive.org/web/20110519083936/http://www.tuaw.com/2007/11/06/quicksilver-goes-open-source/) (2007, archived); [quicksilver/Quicksilver](https://github.com/quicksilver/Quicksilver) on GitHub
- Many Tricks: [Butler](https://manytricks.com/butler/); Peter Maurer: [blog](http://petermaurer.name/blog/); Wired: [Software Makes a Tiger of Panther](https://www.wired.com/2004/07/software-makes-a-tiger-of-panther/) (2004)
- Google Mac Blog: [Search without effort: Quick Search Box](https://googlemac.blogspot.com/2009/01/search-without-effort-quick-search-box.html) (2009); Wikipedia: [Google Quick Search Box](https://en.wikipedia.org/wiki/Google_Quick_Search_Box)
- Running with Crayons: [About Alfred](https://www.alfredapp.com/about/), [Powerpack](https://www.alfredapp.com/powerpack/), [Alfred v2 is here](https://www.alfredapp.com/blog/announcements/alfred-v2-is-here/), [Understanding result ordering](https://www.alfredapp.com/help/kb/understanding-result-ordering/); The Next Web: [Alfred, the personal butler for your Mac, hits version 1.0](https://thenextweb.com/news/alfred-the-personal-butler-for-your-mac-hits-version-1-0-with-new-features); Lifehacker: [Behind the App: The Story of Alfred](https://lifehacker.com/behind-the-app-the-story-of-alfred-1624424125) (2014); Wikipedia: [Alfred](<https://en.wikipedia.org/wiki/Alfred_(software)>)
- Wikipedia: [Spotlight](<https://en.wikipedia.org/wiki/Spotlight_(Apple)>); Apple: [Apple Announces OS X Yosemite](https://www.apple.com/newsroom/2014/06/02Apple-Announces-OS-X-Yosemite/) (2014), [macOS Tahoe 26](https://www.apple.com/newsroom/2025/06/macos-tahoe-26-makes-the-mac-more-capable-productive-and-intelligent-than-ever/) (2025), [Search with Spotlight](https://support.apple.com/guide/mac-help/search-with-spotlight-mchlp1008/mac)
- TechCrunch: [Raycast raises $2.7M](https://techcrunch.com/2020/10/29/developer-productivity-tools-startup-raycast-raises-2-7m-from-accel/) (2020), [Raycast raises $15M](https://techcrunch.com/2021/11/30/developer-productivity-tools-startup-raycast-raises-15m-from-accel-and-coatue/) (2021); Raycast: [developer documentation](https://developers.raycast.com/), [raycast/extensions](https://github.com/raycast/extensions)
