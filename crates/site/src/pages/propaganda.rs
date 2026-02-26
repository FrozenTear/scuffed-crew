use leptos::prelude::*;

#[component]
pub fn PropagandaPage() -> impl IntoView {
    view! {
        <div class="propaganda-page">

            // THE EYE
            <div class="the-eye">
                <div class="eye-icon">"\u{1F441}"</div>
                <span class="eye-label">"They\u{2019}re watching"</span>
            </div>

            // TOP BAR
            <div class="top-bar">
                <div>
                    <span class="status-dot"></span>
                    " CONNECTION ENCRYPTED \u{2014} TLS 1.3 \u{2014} NO COOKIES \u{2014} NO JS TRACKING \u{2014} NO FINGERPRINTING"
                </div>
                <div class="threat-level">"\u{26A0} DISCORD THREAT LEVEL: CRITICAL \u{2014} TEXT SURVEILLANCE ACTIVE"</div>
            </div>

            // HERO
            <section class="hero">
                <div class="container">
                    <div class="hero-warning">"\u{26A0} THIS PAGE DOES NOT TRACK YOU \u{2014} YOUR BROWSER IS CURRENTLY BEING FINGERPRINTED BY 14 OTHER TABS \u{26A0}"</div>
                    <h1>"SCUFFED CREW"</h1>
                    <div class="hero-sub">"They can\u{2019}t track what they can\u{2019}t see"</div>
                    <div class="hero-tagline">
                        "A competitive gaming organization built on a radical premise:"<br/>
                        <span class="hl">"your comms should belong to you, not a data broker."</span><br/><br/>
                        "We play on FACEIT. We coordinate on Matrix. We talk on TeamSpeak."<br/>
                        "Discord is a "<span class="hl-red">"surveillance platform with a chat feature"</span><br/>
                        "and we use it anyway because that\u{2019}s where the people are."<br/>
                        "We just don\u{2019}t keep anything important there."<span class="cursor-blink">"\u{2588}"</span>
                    </div>
                </div>
            </section>

            // LIVE COUNTERS
            <div class="live-counter-bar">
                <div class="counter-item">
                    <span class="counter-value">"~5,438,291"</span>
                    <span class="counter-label">"Discord messages logged since you opened this page"</span>
                </div>
                <div class="counter-item">
                    <span class="counter-value">"600,000,000"</span>
                    <span class="counter-label">"Users exposed in Discord\u{2019}s 2024 data scrape"</span>
                </div>
                <div class="counter-item">
                    <span class="counter-value">"\u{221E}"</span>
                    <span class="counter-label">"Days Discord retains your text messages"</span>
                </div>
            </div>

            // TICKER
            <div class="ticker-wrap">
                <div class="ticker">
                    <span>"\u{26A0} DISCORD TEXT MESSAGES ARE NOT ENCRYPTED \u{2014} DISCORD READS ALL OF THEM"</span>
                    <span>"\u{26A0} DISCORD SHARES DEVICE IDS WITH ADVERTISING PARTNERS"</span>
                    <span>"\u{26A0} 100 THIEVES ADMITTED TO SELLING USER IP ADDRESSES UNDER CA LAW"</span>
                    <span>"\u{26A0} FAZE CLAN PRIVACY POLICY: \u{201C}YOU SHOULD HAVE NO EXPECTATION OF PRIVACY\u{201D}"</span>
                    <span>"\u{26A0} DISCORD AUG 2025: EXPANDED THIRD-PARTY DATA ACQUISITION FOR AD TARGETING"</span>
                    <span>"\u{26A0} DISCORD OCT 2025: DATA BREACH \u{2014} OFFERED VICTIMS 1 YEAR OF EXPERIAN AS COMPENSATION"</span>
                    <span>"\u{26A0} 600 MILLION DISCORD USERS HAD DATA SCRAPED FROM GROUP CHATS IN 2024"</span>
                    <span>"\u{26A0} DISCORD\u{2019}S \u{201C}SCIENCE\u{201D} ENDPOINT TRANSMITS BEHAVIORAL TELEMETRY ON EVERY ACTION YOU TAKE"</span>
                    <span>"\u{26A0} DISCORD REQUIRES MANDATORY AGE VERIFICATION \u{2014} YOUR FACE SCAN ON THEIR SERVERS"</span>
                    <span>"\u{26A0} DISCORD VOICE IS NOW \u{201C}ENCRYPTED\u{201D} BY A PROTOCOL DISCORD WROTE, CONTROLS, AND DELIVERS THE KEYS FOR"</span>
                    // duplicate for seamless scroll
                    <span>"\u{26A0} DISCORD TEXT MESSAGES ARE NOT ENCRYPTED \u{2014} DISCORD READS ALL OF THEM"</span>
                    <span>"\u{26A0} DISCORD SHARES DEVICE IDS WITH ADVERTISING PARTNERS"</span>
                    <span>"\u{26A0} 100 THIEVES ADMITTED TO SELLING USER IP ADDRESSES UNDER CA LAW"</span>
                    <span>"\u{26A0} FAZE CLAN PRIVACY POLICY: \u{201C}YOU SHOULD HAVE NO EXPECTATION OF PRIVACY\u{201D}"</span>
                    <span>"\u{26A0} DISCORD AUG 2025: EXPANDED THIRD-PARTY DATA ACQUISITION FOR AD TARGETING"</span>
                    <span>"\u{26A0} DISCORD OCT 2025: DATA BREACH \u{2014} OFFERED VICTIMS 1 YEAR OF EXPERIAN AS COMPENSATION"</span>
                    <span>"\u{26A0} 600 MILLION DISCORD USERS HAD DATA SCRAPED FROM GROUP CHATS IN 2024"</span>
                    <span>"\u{26A0} DISCORD\u{2019}S \u{201C}SCIENCE\u{201D} ENDPOINT TRANSMITS BEHAVIORAL TELEMETRY ON EVERY ACTION YOU TAKE"</span>
                    <span>"\u{26A0} DISCORD REQUIRES MANDATORY AGE VERIFICATION \u{2014} YOUR FACE SCAN ON THEIR SERVERS"</span>
                    <span>"\u{26A0} DISCORD VOICE IS NOW \u{201C}ENCRYPTED\u{201D} BY A PROTOCOL DISCORD WROTE, CONTROLS, AND DELIVERS THE KEYS FOR"</span>
                </div>
            </div>

            // WHAT THEY HARVEST
            <section>
                <div class="container">
                    <div class="section-label">"// Intelligence Briefing"</div>
                    <div class="section-title">"What Your \u{201C}Free\u{201D} Platform Costs You"</div>
                    <div class="section-body">
                        "Every major esports org runs on Discord. Every one of them has a privacy policy that reads like a warrant. Here\u{2019}s what the platforms powering competitive gaming actually harvest from you \u{2014} not in theory, in their own published policies:"
                    </div>

                    <div class="harvest-grid">
                        <div class="harvest-card">
                            <div class="harvest-icon">"\u{1F4DD}"</div>
                            <h3>"Every Message"</h3>
                            <p>
                                "Discord text is not encrypted. Not DMs. Not private channels. Not \u{201C}members only\u{201D} servers. Every word you type is stored on Discord\u{2019}s servers in plaintext, scanned by automated systems, and retained indefinitely."
                                <a href="https://discord.com/privacy" target="_blank" class="src">"[1]"</a>
                                " They told you this. You clicked \u{201C}I agree.\u{201D}"
                            </p>
                            <div class="harvest-stat">"0%"</div>
                            <div class="harvest-stat-label">"of text messages are encrypted"</div>
                        </div>
                        <div class="harvest-card">
                            <div class="harvest-icon">"\u{1F4CD}"</div>
                            <h3>"Your Behavior"</h3>
                            <p>
                                "What you play. When. How long. Who with. What channels you read. How long you hover. What you click. Discord\u{2019}s /api/v9/science endpoint fires on virtually every user action."
                                <a href="https://discord.com/safety/privacy-preserving-products" target="_blank" class="src">"[2]"</a>
                                " They call it \u{201C}usage statistics.\u{201D} Intelligence agencies call it \u{201C}behavioral profiling.\u{201D}"
                            </p>
                            <div class="harvest-stat">"24/7"</div>
                            <div class="harvest-stat-label">"telemetry while client is running"</div>
                        </div>
                        <div class="harvest-card">
                            <div class="harvest-icon">"\u{1F3AF}"</div>
                            <h3>"Your Identity"</h3>
                            <p>
                                "Aug 2025: Discord now acquires third-party data about you from external sources to build ad profiles."
                                <a href="https://discord.com/safety/important-policy-updates" target="_blank" class="src">"[3]"</a>
                                " 2026: Global age verification rollout \u{2014} submit your face or your ID."
                                <a href="https://discord.com/safety/important-policy-updates" target="_blank" class="src">"[3]"</a>
                                " They\u{2019}re not just logging what you do on Discord. They\u{2019}re buying data about what you do everywhere else."
                            </p>
                            <div class="harvest-stat">"$22B"</div>
                            <div class="harvest-stat-label">"projected comms app advertising market"</div>
                        </div>
                    </div>
                </div>
            </section>

            // DAVE DISSECTION
            <section>
                <div class="container">
                    <div class="section-label">"// Threat Analysis"</div>
                    <div class="section-title">"\u{201C}But Discord Has E2E Encryption Now\u{201D}"</div>
                    <div class="section-body">
                        "Yes. Discord\u{2019}s DAVE protocol encrypts voice and video as of March 2026. Credit where it\u{2019}s due \u{2014} the EFF praised it"
                        <a href="https://www.eff.org/deeplinks/2024/09/discords-end-end-encryption-voice-and-video-step-forward-privacy-all" target="_blank" class="src">"[4]"</a>
                        ", Trail of Bits audited it, it uses MLS, and the whitepaper is public"
                        <a href="https://daveprotocol.com/" target="_blank" class="src">"[5]"</a>
                        ". We\u{2019}re not going to lie to you and say it doesn\u{2019}t exist."<br/><br/>
                        "We\u{2019}re going to explain why it doesn\u{2019}t matter as much as you think."
                    </div>

                    <div class="dave-section">
                        <div class="dave-header">
                            <h3>"DAVE Protocol \u{2014} Autopsy Report"</h3>
                            <div class="dave-verdict">"Verdict: Necessary but insufficient"</div>
                        </div>
                        <div class="dave-body">
                            // Row 1: Voice / Video E2E
                            <div class="dave-row">
                                <div class="dave-feature">"Voice / Video E2E"</div>
                                <div class="dave-status status-good">"YES"</div>
                                <div class="dave-concern">
                                    "Real E2E encryption. MLS-based. Audited by Trail of Bits."
                                    <a href="https://daveprotocol.com/" target="_blank" class="src">"[5]"</a>
                                    " We acknowledge this is legitimate cryptography. Your voice content is protected from eavesdroppers, including Discord employees. This is good."
                                </div>
                            </div>
                            // Row 2: Text Messages E2E
                            <div class="dave-row">
                                <div class="dave-feature">"Text Messages E2E"</div>
                                <div class="dave-status status-bad">"NO"</div>
                                <div class="dave-concern">
                                    "Every text message \u{2014} DMs, private channels, \u{201C}members only\u{201D} servers \u{2014} is stored unencrypted on Discord\u{2019}s servers. Discord explicitly chose not to encrypt text, citing \u{201C}safety tooling.\u{201D}"
                                    <a href="https://support.discord.com/hc/en-us/articles/25968222946071-End-to-End-Encryption-for-Audio-and-Video" target="_blank" class="src">"[6]"</a>
                                    " Your strat discussions, scheduling, and coordination are readable by Discord, by law enforcement with a subpoena, and by anyone who breaches their infrastructure. Which has already happened."
                                </div>
                            </div>
                            // Row 3: MLS Delivery Service
                            <div class="dave-row">
                                <div class="dave-feature">"MLS Delivery Service"</div>
                                <div class="dave-status status-warn">"DISCORD"</div>
                                <div class="dave-concern">
                                    "Discord controls the MLS delivery and authentication service. They facilitate the key exchange. The voice gateway \u{2014} their server \u{2014} selects the DAVE protocol version and manages group epoch transitions."
                                    <a href="https://daveprotocol.com/" target="_blank" class="src">"[5]"</a>
                                    " The cryptography is real, but the infrastructure running it belongs to a company that also runs an advertising pipeline. You\u{2019}re trusting the locksmith not to keep a copy of the key."
                                </div>
                            </div>
                            // Row 4: Voice Metadata
                            <div class="dave-row">
                                <div class="dave-feature">"Voice Metadata"</div>
                                <div class="dave-status status-bad">"EXPOSED"</div>
                                <div class="dave-concern">
                                    "DAVE encrypts the content of your voice. It does not encrypt who is talking, when, for how long, in which channel, with whom, or at what times. Discord\u{2019}s SFU (Selective Forwarding Unit) relays every packet \u{2014} it knows the shape of every conversation even if it can\u{2019}t hear the words. The NSA\u{2019}s former general counsel said it best: \u{201C}We kill people based on metadata.\u{201D}"
                                </div>
                            </div>
                            // Row 5: Stage Channels
                            <div class="dave-row">
                                <div class="dave-feature">"Stage Channels"</div>
                                <div class="dave-status status-bad">"NO E2E"</div>
                                <div class="dave-concern">
                                    "Stage channels are explicitly excluded from DAVE encryption."
                                    <a href="https://discord.com/blog/bringing-dave-to-all-discord-platforms" target="_blank" class="src">"[7]"</a>
                                    " If your org uses stages for announcements, town halls, or community events \u{2014} that\u{2019}s unencrypted. Stream previews are also unencrypted."
                                </div>
                            </div>
                            // Row 6: Client Control
                            <div class="dave-row">
                                <div class="dave-feature">"Client Control"</div>
                                <div class="dave-status status-bad">"CLOSED SOURCE"</div>
                                <div class="dave-concern">
                                    "You cannot verify what Discord\u{2019}s client actually does with your data before encryption or after decryption. The DAVE protocol is open. The whitepaper is public. The client that implements it is proprietary. You are trusting closed-source software with your microphone input. Third-party clients are banned by Discord\u{2019}s ToS."
                                    <a href="https://discord.com/terms" target="_blank" class="src">"[8]"</a>
                                </div>
                            </div>
                            // Row 7: Process Scanning
                            <div class="dave-row">
                                <div class="dave-feature">"Process Scanning"</div>
                                <div class="dave-status status-bad">"ACTIVE"</div>
                                <div class="dave-concern">
                                    "While your voice is encrypted, Discord\u{2019}s client continues to scan your running processes for \u{201C}activity status,\u{201D}"
                                    <a href="https://discord.com/privacy" target="_blank" class="src">"[1]"</a>
                                    " collect system telemetry via the science endpoint, fingerprint your device, and phone home to sentry.io. The mic is encrypted. Everything else about your system is not."
                                </div>
                            </div>
                        </div>
                    </div>

                    // DAVE Summary Box
                    <div style="margin-top: 1.5rem; padding: 1.5rem; border: 1px solid rgba(255,170,0,0.15); border-radius: 2px; background: var(--amber-soft);">
                        <div style="font-family: 'Share Tech Mono', monospace; font-size: 0.55rem; letter-spacing: 2px; text-transform: uppercase; color: var(--amber); margin-bottom: 0.75rem;">"// Summary"</div>
                        <div style="font-size: 0.75rem; color: var(--text); line-height: 1.9;">
                            "DAVE is a lock on one window of a house where the front door is wide open, the walls are made of glass, and the landlord has cameras in every room. They encrypted the one thing they couldn\u{2019}t monetize (voice) and left everything they can monetize (text, metadata, behavior, identity) completely exposed. Congratulations on your encrypted callouts. They still know who you called, when, for how long, what game you were playing, what you typed before and after, and what you had for breakfast based on the DoorDash link you pasted in #general."
                        </div>
                    </div>
                </div>
            </section>

            // ASCII DIVIDER
            <div class="ascii-divider">
                "\u{2554}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2557}"<br/>
                "\u{2551}\u{00A0}\u{00A0}THEY ENCRYPTED YOUR VOICE BECAUSE VOICE DATA WAS A LIABILITY.\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{2551}"<br/>
                "\u{2551}\u{00A0}\u{00A0}THEY LEFT TEXT OPEN BECAUSE TEXT DATA IS AN ASSET.\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{2551}"<br/>
                "\u{2551}\u{00A0}\u{00A0}THIS IS NOT A PRIVACY FEATURE. THIS IS RISK MANAGEMENT.\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{2551}"<br/>
                "\u{255A}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{2550}\u{255D}"
            </div>

            // METADATA KILLS
            <section>
                <div class="container">
                    <div class="section-label">"// The Thing Nobody Talks About"</div>
                    <div class="section-title">"Content Is a Distraction. Metadata Is the Weapon."</div>
                    <div class="section-body">
                        "Everyone argues about whether messages are encrypted. Almost nobody talks about metadata \u{2014} the data about the data. Here\u{2019}s what metadata reveals about your gaming life on Discord vs. on our stack:"
                    </div>

                    <div class="metadata-visual">
                        <div class="metadata-box" style="border-color: rgba(255,0,64,0.15);">
                            <h4 style="color: var(--red);">"Discord Knows"</h4>
                            <div class="meta-line">
                                <span class="meta-key">"who_spoke_to:"</span>" "<span class="meta-val-bad">"[Nightfall#4821, xRaven#1337, ...]"</span><br/>
                                <span class="meta-key">"channel:"</span>" "<span class="meta-val-bad">"#blackwatch-strats"</span><br/>
                                <span class="meta-key">"server:"</span>" "<span class="meta-val-bad">"Scuffed Crew (id: 1098234871234)"</span><br/>
                                <span class="meta-key">"duration:"</span>" "<span class="meta-val-bad">"2h 41m 13s"</span><br/>
                                <span class="meta-key">"participants:"</span>" "<span class="meta-val-bad">"6"</span><br/>
                                <span class="meta-key">"time:"</span>" "<span class="meta-val-bad">"2026-02-25 19:00-21:41 CET"</span><br/>
                                <span class="meta-key">"game_detected:"</span>" "<span class="meta-val-bad">"Overwatch 2 (pid: 28441)"</span><br/>
                                <span class="meta-key">"device:"</span>" "<span class="meta-val-bad">"Linux x86_64, Intel Arc A770"</span><br/>
                                <span class="meta-key">"ip_address:"</span>" "<span class="meta-val-bad">"85.xxx.xxx.xxx (Oslo, NO)"</span><br/>
                                <span class="meta-key">"client:"</span>" "<span class="meta-val-bad">"Discord 0.0.71 (electron)"</span><br/>
                                <span class="meta-key">"mic_active:"</span>" "<span class="meta-val-bad">"true (intervals logged)"</span><br/>
                                <span class="meta-key">"text_before_call:"</span>" "<span class="meta-val-bad">"\u{201C}ok lets run dive comp tonight\u{201D}"</span><br/>
                                <span class="meta-key">"text_after_call:"</span>" "<span class="meta-val-bad">"\u{201C}gg, same time thursday?\u{201D}"</span><br/>
                                <span class="meta-key">"voice_content:"</span>" "<span class="meta-val-good">"[DAVE ENCRYPTED \u{2014} CANNOT READ]"</span><br/>
                            </div>
                        </div>
                        <div class="metadata-box" style="border-color: rgba(0,255,65,0.12);">
                            <h4 style="color: var(--green);">"Our TeamSpeak Server Knows"</h4>
                            <div class="meta-line">
                                <span class="meta-key">"who_connected:"</span>" "<span class="meta-val-good">"[username only, no external ID]"</span><br/>
                                <span class="meta-key">"channel:"</span>" "<span class="meta-val-good">"Blackwatch"</span><br/>
                                <span class="meta-key">"duration:"</span>" "<span class="meta-val-good">"[logged if we choose, we don\u{2019}t]"</span><br/>
                                <span class="meta-key">"participants:"</span>" "<span class="meta-val-good">"[visible only to other users]"</span><br/>
                                <span class="meta-key">"game_detected:"</span>" "<span class="meta-val-good">"[no process scanning]"</span><br/>
                                <span class="meta-key">"device:"</span>" "<span class="meta-val-good">"[not collected]"</span><br/>
                                <span class="meta-key">"ip_address:"</span>" "<span class="meta-val-good">"[server sees IP, we don\u{2019}t log it]"</span><br/>
                                <span class="meta-key">"telemetry:"</span>" "<span class="meta-val-good">"[none \u{2014} open source, verified]"</span><br/>
                                <span class="meta-key">"text_context:"</span>" "<span class="meta-val-good">"[lives on Matrix, E2E encrypted]"</span><br/>
                                <span class="meta-key">"voice_content:"</span>" "<span class="meta-val-good">"[AES-256 encrypted to our server]"</span><br/>
                                <span class="meta-key">"third_party_access:"</span>" "<span class="meta-val-good">"[zero \u{2014} we own the hardware]"</span><br/>
                                <span class="meta-key">"data_sold_to:"</span>" "<span class="meta-val-good">"[nobody \u{2014} we don\u{2019}t have a shop]"</span><br/>
                            </div>
                        </div>
                    </div>

                    <div style="margin-top: 1.5rem; font-family: 'Share Tech Mono', monospace; font-size: 0.6rem; color: var(--text-dim); letter-spacing: 1px; text-align: center;">
                        "Discord encrypted one field. Everything else is wide open."<br/>
                        "We encrypted everything. And we don\u{2019}t even log most of it."
                    </div>
                </div>
            </section>

            // BREACH TIMELINE
            <section>
                <div class="container">
                    <div class="section-label">"// Incident History"</div>
                    <div class="section-title">"Discord\u{2019}s Track Record"</div>
                    <div class="section-body">
                        "Trust is earned, not declared. Here\u{2019}s what the platform you\u{2019}re trusting with your comms has done recently:"
                    </div>

                    <div class="timeline">
                        <div class="timeline-event">
                            <div class="timeline-date">"2024 \u{2014} Mass Data Scrape"</div>
                            <div class="timeline-text">
                                "600 million Discord users had data scraped from group chats across thousands of servers."
                                <a href="https://www.pcworld.com/article/2461674/discord-starts-with-full-range-encryption-for-voice-and-video-calls.html" target="_blank" class="src">"[10]"</a>
                                " Usernames, message content, server memberships \u{2014} harvested at scale. Discord\u{2019}s response: silence, then downplaying."
                            </div>
                        </div>
                        <div class="timeline-event">
                            <div class="timeline-date">"October 2025 \u{2014} Data Breach"</div>
                            <div class="timeline-text">
                                "Discord suffered a confirmed data breach. Their compensation to affected users: one year of Experian Identity Plus membership."
                                <a href="https://piunikaweb.com/2026/02/10/discord-voice-cutoff-old-clients-march-2026/" target="_blank" class="src">"[11]"</a>
                                " Your data is now in a breach database forever. They gave you twelve months of credit monitoring. Seems fair."
                            </div>
                        </div>
                        <div class="timeline-event">
                            <div class="timeline-date">"August 2025 \u{2014} Policy Expansion"</div>
                            <div class="timeline-text">
                                "Discord updated Terms of Service and Privacy Policy to expand third-party data acquisition."
                                <a href="https://discord.com/safety/important-policy-updates" target="_blank" class="src">"[3]"</a>
                                " They now buy data about you from external sources to \u{201C}personalize sponsored content.\u{201D} They\u{2019}re not just watching what you do on Discord \u{2014} they\u{2019}re buying your profile from data brokers to follow you everywhere."
                            </div>
                        </div>
                        <div class="timeline-event">
                            <div class="timeline-date">"2025-2026 \u{2014} Mandatory Age Verification"</div>
                            <div class="timeline-text">
                                "Global rollout of age verification."
                                <a href="https://discord.com/safety/important-policy-updates" target="_blank" class="src">"[3]"</a>
                                " Submit government ID or a face scan to prove you\u{2019}re human. This biometric data is now on Discord\u{2019}s servers \u{2014} servers that have already been breached once. Your face. On the servers of a company whose previous breach mitigation was an Experian subscription."
                            </div>
                        </div>
                        <div class="timeline-event">
                            <div class="timeline-date">"March 2026 \u{2014} DAVE Mandate"</div>
                            <div class="timeline-text">
                                "All voice must use DAVE encryption. Older clients blocked."
                                <a href="https://discord.com/blog/bringing-dave-to-all-discord-platforms" target="_blank" class="src">"[7]"</a>
                                " Third-party clients must implement DAVE or die."
                                <a href="https://support.discord.com/hc/en-us/articles/38025123604631-Minimum-Client-Version-Requirements-for-Voice-Chat" target="_blank" class="src">"[12]"</a>
                                " Discord frames this as privacy. It also means they control the only clients allowed to connect to their voice infrastructure. The walls of the garden just got higher."
                            </div>
                        </div>
                    </div>
                </div>
            </section>

            // COMPARE TABLE
            <section>
                <div class="container">
                    <div class="section-label">"// Comparative Analysis"</div>
                    <div class="section-title">"How Gaming Orgs Treat Your Data"</div>

                    <table class="compare-table">
                        <thead>
                            <tr>
                                <th>"Organization"</th>
                                <th>"Sells Data"</th>
                                <th>"Tracking"</th>
                                <th>"Text E2E"</th>
                                <th>"Voice E2E"</th>
                                <th>"Self-Hosted"</th>
                            </tr>
                        </thead>
                        <tbody>
                            <tr>
                                <td class="platform-name">"FaZe Clan "<a href="https://shop.fazeclan.com/pages/privacy-policy" target="_blank" class="src">"[9]"</a></td>
                                <td class="status-bad">"YES"</td>
                                <td class="status-bad">"Beacons, pixels"</td>
                                <td class="status-bad">"NO"</td>
                                <td class="status-warn">"DAVE*"</td>
                                <td class="status-bad">"NO"</td>
                            </tr>
                            <tr>
                                <td class="platform-name">"100 Thieves "<a href="https://100thieves.com/pages/privacy-policy" target="_blank" class="src">"[13]"</a></td>
                                <td class="status-bad">"YES (CA def.)"</td>
                                <td class="status-bad">"IP + device IDs"</td>
                                <td class="status-bad">"NO"</td>
                                <td class="status-warn">"DAVE*"</td>
                                <td class="status-bad">"NO"</td>
                            </tr>
                            <tr>
                                <td class="platform-name">"Team Liquid"</td>
                                <td class="status-warn">"UNCLEAR"</td>
                                <td class="status-bad">"Cookies + 3rd party"</td>
                                <td class="status-bad">"NO"</td>
                                <td class="status-warn">"DAVE*"</td>
                                <td class="status-bad">"NO"</td>
                            </tr>
                            <tr>
                                <td class="platform-name">"Any Discord Clan"</td>
                                <td class="status-warn">"N/A"</td>
                                <td class="status-bad">"Full Discord stack"</td>
                                <td class="status-bad">"NO"</td>
                                <td class="status-warn">"DAVE*"</td>
                                <td class="status-bad">"NO"</td>
                            </tr>
                            <tr style="background: var(--green-dark);">
                                <td class="platform-name" style="color: var(--green);">"SCUFFED CREW"</td>
                                <td class="status-good">"NO"</td>
                                <td class="status-good">"NONE"</td>
                                <td class="status-good">"MEGOLM E2E"</td>
                                <td class="status-good">"SELF-HOSTED"</td>
                                <td class="status-good">"YES"</td>
                            </tr>
                        </tbody>
                    </table>
                    <div style="margin-top: 0.75rem; font-family: 'Share Tech Mono', monospace; font-size: 0.5rem; color: var(--text-dim); letter-spacing: 1px;">
                        "* DAVE encrypts voice content only. Text, metadata, behavioral telemetry, device fingerprints, and process lists remain fully exposed. Discord controls MLS key delivery infrastructure."
                    </div>
                </div>
            </section>

            // MANIFESTO
            <section>
                <div class="container">
                    <div class="manifesto">
                        <p>
                            "We don\u{2019}t read your messages. We can\u{2019}t \u{2014} they\u{2019}re encrypted."<br/>
                            "We don\u{2019}t log your voice. We can\u{2019}t \u{2014} we don\u{2019}t store it."<br/>
                            "We don\u{2019}t scan your processes. We don\u{2019}t have a client on your machine."<br/>
                            "We don\u{2019}t sell your IP. We don\u{2019}t have a merch store with tracking pixels."<br/>
                            "We don\u{2019}t require your phone number. Or your face. Or your government ID."<br/>
                            "We don\u{2019}t buy data about you from third-party brokers."<br/>
                            "We don\u{2019}t compensate breaches with twelve months of Experian."<br/>
                            "We don\u{2019}t have breaches. We don\u{2019}t store anything worth breaching."<br/><br/>
                            "We just play the game."<br/>
                        </p>
                        <div class="attribution">"\u{2014} Scuffed Crew Founding Principles, 2026"</div>
                    </div>
                </div>
            </section>

            // COMMS ARCHITECTURE — THE FORTRESS
            <section>
                <div class="container">
                    <div class="section-label">"// Communications Architecture"</div>
                    <div class="section-title">"The Fortress"</div>
                    <div class="section-body">
                        "Three layers. One is a necessary evil. One is a partial concession. Two are hardened by design. The important stuff never touches infrastructure we don\u{2019}t own."
                    </div>

                    <div class="fortress-stack">
                        // Layer 1: Discord
                        <div class="fortress-layer">
                            <div class="fortress-layer-badge badge-honeypot">"\u{26A0} HOSTILE TERRITORY \u{2014} TEXT FULLY MONITORED \u{2014} VOICE ENCRYPTED BY LANDLORD"</div>
                            <h3>"Discord \u{2014} The Honeypot"</h3>
                            <p>
                                "Yes, we have a Discord. It\u{2019}s the recruitment layer. Announcements, casual chat, LFG for pickups. Treat it like a public square \u{2014} fine for meeting people, terrible for discussing anything operational. Every text message is stored in plaintext on Discord\u{2019}s servers."
                                <a href="https://discord.com/privacy" target="_blank" class="src">"[1]"</a>
                                " Voice is DAVE-encrypted, which means Discord can\u{2019}t hear you, but they still know who you\u{2019}re talking to, when, and for how long."
                                <a href="https://daveprotocol.com/" target="_blank" class="src">"[5]"</a>
                                " It\u{2019}s the difference between wiretapping and traffic analysis. Intelligence agencies have killed people with less than what Discord\u{2019}s metadata provides for free."
                            </p>
                            <span class="encryption-tag-bad">"TEXT: UNENCRYPTED \u{2014} DISCORD READS EVERYTHING"</span>
                            <span class="encryption-tag-warn">"VOICE: DAVE E2E \u{2014} CONTENT ENCRYPTED, METADATA EXPOSED \u{2014} KEY DELIVERY BY DISCORD"</span>
                            <span class="encryption-tag-bad">"TELEMETRY: ACTIVE \u{2014} SCIENCE ENDPOINT, PROCESS SCAN, DEVICE FINGERPRINT"</span>
                        </div>
                        // Layer 2: Matrix
                        <div class="fortress-layer">
                            <div class="fortress-layer-badge badge-secure">"\u{1F510} VERIFIED SECURE \u{2014} SELF-HOSTED \u{2014} E2E ENCRYPTED \u{2014} FEDERATED"</div>
                            <h3>"Matrix / Element \u{2014} The War Room"</h3>
                            <p>
                                "When you make a roster, you get access. Per-team rooms for strats, scheduling, org coordination. Self-hosted on hardware we control. End-to-end encrypted by default \u{2014} not \u{201C}encrypted by a protocol the platform wrote and manages the key exchange for,\u{201D} actually encrypted, with keys that never leave your devices. Even if someone seized our server tomorrow, they\u{2019}d get ciphertext. The protocol is federated \u{2014} if Element disappears, the network lives on. If we disappear, you export your keys and keep your history. Try that with Discord."
                            </p>
                            <span class="encryption-tag">"ENCRYPTION: OLMA/MEGOLM E2E \u{2014} KEYS NEVER LEAVE YOUR DEVICE \u{2014} SELF-HOSTED \u{2014} ZERO THIRD-PARTY ACCESS"</span>
                        </div>
                        // Layer 3: TeamSpeak
                        <div class="fortress-layer">
                            <div class="fortress-layer-badge badge-secure">"\u{1F510} VERIFIED SECURE \u{2014} SELF-HOSTED \u{2014} OPEN SOURCE \u{2014} AUDITABLE"</div>
                            <h3>"TeamSpeak \u{2014} The Bunker"</h3>
                            <p>
                                "Voice comms. Self-hosted. The same platform competitive CS and esports teams have trusted for two decades \u{2014} except we run the server, not a hosting company. Your voice goes from your mic to our hardware to your teammates\u{2019} ears. No SFU controlled by an ad company. No MLS key exchange managed by a corporation that just expanded its data brokerage relationships. No process scanning. No behavioral telemetry. No closed-source client you\u{2019}re forbidden from modifying. Just voice. Encrypted. On our metal."
                            </p>
                            <span class="encryption-tag">"ENCRYPTION: AES-256 \u{2014} SELF-HOSTED \u{2014} ZERO TELEMETRY \u{2014} 20 YEARS OF COMPETITIVE TRUST"</span>
                        </div>
                    </div>
                </div>
            </section>

            // LOG FEED
            <section>
                <div class="container">
                    <div class="section-label">"// Live Feed"</div>
                    <div class="section-title">"What Our Firewall Looks Like"</div>
                    <div class="log-feed">
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:02]"</span>" "<span class="blocked">"BLOCKED"</span>" Discord telemetry \u{2192} sentry.io/api/discord"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:03]"</span>" "<span class="blocked">"BLOCKED"</span>" Discord science \u{2192} /api/v9/science (behavioral payload: 4.2KB)"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:03]"</span>" "<span class="action">"ALLOWED"</span>" Matrix sync \u{2192} matrix.scuffedcrew.gg (E2E verified, 0 plaintext)"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:04]"</span>" "<span class="blocked">"BLOCKED"</span>" Discord fingerprint \u{2192} /api/v9/auth/fingerprint"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:04]"</span>" "<span class="blocked">"BLOCKED"</span>" Google Analytics \u{2192} google-analytics.com/collect"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:05]"</span>" "<span class="action">"ALLOWED"</span>" TeamSpeak voice \u{2192} ts.scuffedcrew.gg:9987 (AES-256)"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:06]"</span>" "<span class="blocked">"BLOCKED"</span>" Discord process scan \u{2192} /api/v9/users/@me/activities"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:06]"</span>" "<span class="suspect">"SUSPECT"</span>" Discord DAVE key exchange \u{2192} voice-gateway (MLS epoch transition)"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:07]"</span>" "<span class="blocked">"BLOCKED"</span>" Discord ad targeting \u{2192} cdn.discordapp.com/sponsored"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:07]"</span>" "<span class="blocked">"BLOCKED"</span>" Meta pixel \u{2192} connect.facebook.net/signals"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:08]"</span>" "<span class="action">"ALLOWED"</span>" FACEIT match lobby \u{2192} faceit.com/en/ow2 (no client required)"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:09]"</span>" "<span class="blocked">"BLOCKED"</span>" Discord metrics \u{2192} /api/v9/metrics (payload: system info, GPU, RAM)"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:09]"</span>" "<span class="action">"ALLOWED"</span>" Matrix room event \u{2192} #blackwatch:scuffedcrew.gg (E2E, Megolm)"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:10]"</span>" "<span class="blocked">"BLOCKED"</span>" Discord Krisp telemetry \u{2192} krisp.ai/api/v1/telemetry"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:11]"</span>" "<span class="blocked">"BLOCKED"</span>" Discord age verification \u{2192} discord.com/api/v9/verification"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:11]"</span>" "<span class="action">"ALLOWED"</span>" TeamSpeak keepalive \u{2192} ping OK (11ms, 0 metadata leaked)"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:12]"</span>" "<span class="blocked">"BLOCKED"</span>" Discord third-party data ingest \u{2192} /api/v9/users/@me/affinities"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:13]"</span>" "<span class="action">"ALLOWED"</span>" Matrix key verification \u{2192} cross-sign OK (ed25519)"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:14]"</span>" "<span class="blocked">"BLOCKED"</span>" Discord quest targeting \u{2192} /api/v9/users/@me/billing/quests"</div>
                        <div class="log-line"><span class="timestamp">"[2026-02-25 19:41:14]"</span>" "<span class="action">"ALLOWED"</span>" FACEIT API \u{2192} open.faceit.com/data/v4/ (public stats only)"</div>
                    </div>
                </div>
            </section>

            // RECRUITMENT
            <section>
                <div class="container">
                    <div class="section-label">"// Recruitment Protocol"</div>
                    <div class="section-title">"What We Actually Need From You"</div>
                    <div class="section-body">
                        "Most orgs want your email, phone, social handles, real name, and now apparently your face. We need five things:"
                    </div>

                    <ul class="req-list">
                        <li>
                            <span class="req-icon req-mandatory">"!"</span>
                            <span><strong style="color: var(--text-bright);">"A username."</strong>" Not your real name. We don\u{2019}t want it. If you tell us, we\u{2019}ll forget it on purpose."</span>
                        </li>
                        <li>
                            <span class="req-icon req-mandatory">"!"</span>
                            <span><strong style="color: var(--text-bright);">"Your game + role."</strong>" So we know where to slot you. That\u{2019}s it."</span>
                        </li>
                        <li>
                            <span class="req-icon req-mandatory">"!"</span>
                            <span><strong style="color: var(--text-bright);">"Your timezone + availability."</strong>" One rule: communicate. We need to know when you can play."</span>
                        </li>
                        <li>
                            <span class="req-icon req-mandatory">"!"</span>
                            <span><strong style="color: var(--text-bright);">"A mic."</strong>" We need callouts. We don\u{2019}t need them routed through an ad company\u{2019}s infrastructure."</span>
                        </li>
                        <li>
                            <span class="req-icon req-mandatory">"!"</span>
                            <span><strong style="color: var(--text-bright);">"Willingness to install Element and TeamSpeak."</strong>" Two apps. Both free. Both lighter than the Electron panopticon Discord calls a \u{201C}client.\u{201D} If this is a dealbreaker, we respect that \u{2014} but this isn\u{2019}t the org for you."</span>
                        </li>
                    </ul>

                    // "What we will never ask for" box
                    <div style="margin-top: 2.5rem; padding: 1.5rem; background: var(--bg-card); border: 1px solid var(--border); border-radius: 2px;">
                        <div style="font-family: 'Share Tech Mono', monospace; font-size: 0.5rem; letter-spacing: 2px; text-transform: uppercase; color: var(--green); margin-bottom: 0.75rem;">"What we will never ask for"</div>
                        <div style="font-size: 0.72rem; color: var(--text); line-height: 2;">
                            "Your real name \u{00B7} Your email address \u{00B7} Your phone number \u{00B7} Your social media \u{00B7} Your location \u{00B7} Your age (18+ honor system) \u{00B7} Your "
                            <span class="redacted">"government identification"</span>
                            " \u{00B7} Your "
                            <span class="redacted">"biometric face scan"</span>
                            " \u{00B7} Your "
                            <span class="redacted">"mother\u{2019}s maiden name"</span>
                            " \u{00B7} Your "
                            <span class="redacted">"running process list"</span>
                            " \u{00B7} Access to your contacts \u{00B7} Permission to scan your system \u{00B7} A mandatory closed-source client that phones home to an ad network every 30 seconds \u{00B7} Twelve months of Experian when we inevitably lose all of the above"
                        </div>
                    </div>

                    // PARANOIA METER
                    <div class="paranoia-meter">
                        <h4>"Current Organizational Paranoia Level"</h4>
                        <div class="paranoia-bar"><div class="paranoia-fill"></div></div>
                        <div class="paranoia-labels">
                            <span>"Na\u{00EF}ve"</span>
                            <span>"Cautious"</span>
                            <span>"Informed"</span>
                            <span>"Paranoid"</span>
                            <span>"Correct"</span>
                        </div>
                        <div class="paranoia-current">"Correct"</div>
                        <div class="paranoia-sub">"It\u{2019}s not paranoia if they published it in their own privacy policy."</div>
                    </div>
                </div>
            </section>

            // JOIN CTA
            <section class="join-section">
                <div class="container">
                    <div class="section-label" style="margin-bottom: 2rem;">"// Enlist"</div>
                    <a href="#" class="join-btn">"JOIN SCUFFED CREW"</a>
                    <div class="join-disclaimer">
                        "Clicking this button takes you to our Discord. Yes, the irony is noted."<br/>
                        "Think of it as going behind enemy lines to recruit assets."<br/>
                        "The real comms happen on infrastructure we own."<br/>
                        "Discord gets to know you joined. They don\u{2019}t get to know what happens next."<br/><br/>
                        "(Use a VPN. Obviously.)"
                    </div>
                </div>
            </section>

            // BARCODE
            <div class="barcode-section">
                <div class="barcode">"SCUFFED-CREW-2026-PRIVACY-IS-NOT-PARANOIA-ITS-READING-COMPREHENSION"</div>
                <div class="barcode-label">"You are not a product. You are not a data point. You are a player. Act like it."</div>
            </div>

            // SOURCES
            <section>
                <div class="container">
                    <div class="sources-section">
                        <h4>"// Sources \u{2014} We Don\u{2019}t Make This Up, They Published It Themselves"</h4>
                        <div class="source-item"><span class="source-id">"[1]"</span>" Discord Privacy Policy \u{2014} "<a href="https://discord.com/privacy" target="_blank">"discord.com/privacy"</a></div>
                        <div class="source-item"><span class="source-id">"[2]"</span>" Discord Privacy Preserving Products \u{2014} "<a href="https://discord.com/safety/privacy-preserving-products" target="_blank">"discord.com/safety/privacy-preserving-products"</a></div>
                        <div class="source-item"><span class="source-id">"[3]"</span>" Discord Policy Updates August 2025 \u{2014} "<a href="https://discord.com/safety/important-policy-updates" target="_blank">"discord.com/safety/important-policy-updates"</a></div>
                        <div class="source-item"><span class="source-id">"[4]"</span>" EFF: Discord\u{2019}s E2E Encryption \u{2014} A Step Forward \u{2014} "<a href="https://www.eff.org/deeplinks/2024/09/discords-end-end-encryption-voice-and-video-step-forward-privacy-all" target="_blank">"eff.org/deeplinks/2024/09/..."</a></div>
                        <div class="source-item"><span class="source-id">"[5]"</span>" DAVE Protocol Whitepaper \u{2014} "<a href="https://daveprotocol.com/" target="_blank">"daveprotocol.com"</a></div>
                        <div class="source-item"><span class="source-id">"[6]"</span>" Discord: E2E Encryption for Audio and Video \u{2014} "<a href="https://support.discord.com/hc/en-us/articles/25968222946071-End-to-End-Encryption-for-Audio-and-Video" target="_blank">"support.discord.com/hc/en-us/articles/25968222946071"</a></div>
                        <div class="source-item"><span class="source-id">"[7]"</span>" Discord: Bringing DAVE to All Platforms \u{2014} "<a href="https://discord.com/blog/bringing-dave-to-all-discord-platforms" target="_blank">"discord.com/blog/bringing-dave-to-all-discord-platforms"</a></div>
                        <div class="source-item"><span class="source-id">"[8]"</span>" Discord Terms of Service \u{2014} "<a href="https://discord.com/terms" target="_blank">"discord.com/terms"</a></div>
                        <div class="source-item"><span class="source-id">"[9]"</span>" FaZe Clan Privacy Policy \u{2014} "<a href="https://shop.fazeclan.com/pages/privacy-policy" target="_blank">"shop.fazeclan.com/pages/privacy-policy"</a></div>
                        <div class="source-item"><span class="source-id">"[10]"</span>" PCWorld: Discord Encryption & 600M Data Scrape \u{2014} "<a href="https://www.pcworld.com/article/2461674/discord-starts-with-full-range-encryption-for-voice-and-video-calls.html" target="_blank">"pcworld.com/article/2461674"</a></div>
                        <div class="source-item"><span class="source-id">"[11]"</span>" PiunikaWeb: Discord Voice Cutoff & Breach \u{2014} "<a href="https://piunikaweb.com/2026/02/10/discord-voice-cutoff-old-clients-march-2026/" target="_blank">"piunikaweb.com/2026/02/10/..."</a></div>
                        <div class="source-item"><span class="source-id">"[12]"</span>" Discord: Minimum Client Version Requirements \u{2014} "<a href="https://support.discord.com/hc/en-us/articles/38025123604631-Minimum-Client-Version-Requirements-for-Voice-Chat" target="_blank">"support.discord.com/hc/en-us/articles/38025123604631"</a></div>
                        <div class="source-item"><span class="source-id">"[13]"</span>" 100 Thieves Privacy Policy \u{2014} "<a href="https://100thieves.com/pages/privacy-policy" target="_blank">"100thieves.com/pages/privacy-policy"</a></div>
                    </div>
                    <div style="margin-top: 1rem; font-family: 'Share Tech Mono', monospace; font-size: 0.5rem; color: var(--text-dim); letter-spacing: 1px; text-align: center;">
                        "Every claim on this page links to its primary source. We didn\u{2019}t editorialize the policies. We read them. You should too."
                    </div>
                </div>
            </section>

            // FOOTER
            <footer>
                <div class="footer-logo">"SCUFFED CREW"</div>
                <div class="footer-text">
                    "This website stores zero cookies. Runs zero analytics. Contains zero tracking pixels."<br/>
                    "No JavaScript telemetry. No fingerprinting. No third-party resources."<br/>
                    "Hosted on static infrastructure. Source available on request."<br/>
                    "Built with Zola. Served over HTTPS. Nothing else."<br/><br/>
                    "We don\u{2019}t know you visited this page. That\u{2019}s the point."
                </div>
                <div class="footer-warning">
                    "\u{26A0} THIS SITE RESPECTS YOUR PRIVACY \u{2014} YOUR ISP DOESN\u{2019}T \u{2014} DISCORD DOESN\u{2019}T \u{2014} YOUR BROWSER DOESN\u{2019}T \u{2014} USE A VPN AND HARDEN YOUR SETUP \u{26A0}"
                </div>
                <div class="footer-tinfoil">
                    "If you\u{2019}ve read this far, you\u{2019}re either one of us or you\u{2019}re compiling a dossier."<br/>
                    "Either way, we didn\u{2019}t log your visit. Good luck proving you were here."<br/><br/>
                    "P.S. \u{2014} Check your Discord client\u{2019}s network traffic sometime. Bring a drink. You\u{2019}ll need it."
                </div>
            </footer>

        </div>
    }
}
