use leptos::prelude::*;
use crate::components::SectionHeader;

#[component]
pub fn Comms() -> impl IntoView {
    view! {
        <section id="comms" class="section-comms-bg">
            <SectionHeader
                label="// Communication"
                title="How we talk"
                color="blue"
                description="Matrix for text, TeamSpeak for voice. Self-hosted, no middleman."
            />

            <div class="comms-grid">
                <div class="comm-card comm-card-discord" data-reveal="" data-reveal-delay="1">
                    <div class="comm-card-header">
                        <div class="comm-icon-svg">
                            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
                            </svg>
                        </div>
                        <h3>"Matrix (Commet)"</h3>
                    </div>
                    <p>"All text lives here \u{2014} announcements, scheduling, casual chat, recruitment. Self-hosted on our server. Use Commet on desktop, Element X on your phone. Same account, same rooms."</p>
                    <div class="comm-tags">
                        <span class="comm-tag comm-public">"Open to all"</span>
                    </div>
                </div>

                <div class="comm-card comm-card-ts" data-reveal="" data-reveal-delay="2">
                    <div class="comm-card-header">
                        <div class="comm-icon-svg">
                            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                <path d="M3 18v-6a9 9 0 0 1 18 0v6"/>
                                <path d="M21 19a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3z"/>
                                <path d="M3 19a2 2 0 0 0 2 2h1a2 2 0 0 0 2-2v-3a2 2 0 0 0-2-2H3z"/>
                            </svg>
                        </div>
                        <h3>"TeamSpeak"</h3>
                    </div>
                    <p>"Voice comms for play nights and scrims. Self-hosted, low latency, no distractions mid-match. Been the competitive standard for two decades."</p>
                    <div class="comm-tags">
                        <span class="comm-tag comm-members">"Rostered members"</span>
                    </div>
                </div>
            </div>

            // Why Matrix section
            <div class="why-matrix" data-reveal="" data-reveal-delay="3">
                <div class="why-matrix-header">"Why Matrix instead of Discord?"</div>
                <div class="why-matrix-body">
                    <p>"We use Matrix because we\u{2019}d rather own our infrastructure than rent it. Our server, our data, our rules. We control the uptime, the backups, and what happens to the community long-term."</p>
                    <p>"Commet gives you the same UI you\u{2019}re used to from Discord \u{2014} channels, voice, roles, emoji \u{2014} plus built-in calendar rooms and shared photo albums that Discord doesn\u{2019}t have."</p>

                    <div class="why-tradeoffs">
                        <div class="why-tradeoff">
                            <h4>"What you gain"</h4>
                            <div class="why-tradeoff-item"><span class="marker">"+"</span>" Self-hosted \u{2014} we own the server and data"</div>
                            <div class="why-tradeoff-item"><span class="marker">"+"</span>" E2E encrypted messages, files, and calls"</div>
                            <div class="why-tradeoff-item"><span class="marker">"+"</span>" Built-in calendar rooms with .ics sync"</div>
                            <div class="why-tradeoff-item"><span class="marker">"+"</span>" No account on a third-party platform required"</div>
                            <div class="why-tradeoff-item"><span class="marker">"+"</span>" Community lives on our hardware \u{2014} not rented space"</div>
                        </div>
                        <div class="why-tradeoff">
                            <h4>"What you give up"</h4>
                            <div class="why-tradeoff-item"><span class="marker">"\u{2212}"</span>" Smaller bot and integration ecosystem"</div>
                            <div class="why-tradeoff-item"><span class="marker">"\u{2212}"</span>" Fewer people already have an account"</div>
                            <div class="why-tradeoff-item"><span class="marker">"\u{2212}"</span>" Screen sharing is less polished"</div>
                            <div class="why-tradeoff-item"><span class="marker">"\u{2212}"</span>" Search isn\u{2019}t as refined yet"</div>
                            <div class="why-tradeoff-item"><span class="marker">"\u{2212}"</span>" Newer platform \u{2014} still growing"</div>
                        </div>
                    </div>
                </div>
            </div>
        </section>
    }
}
