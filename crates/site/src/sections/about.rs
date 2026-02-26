use leptos::prelude::*;
use crate::components::SectionHeader;

#[component]
pub fn About() -> impl IntoView {
    view! {
        <section id="about">
            <SectionHeader
                label="// The Ethos"
                title="Not a server. A clan."
                color="purple"
                description="A structured gaming org with game-specific squads and scheduled play nights. No drama. Life comes first \u{2014} the games come second, but we still show up."
            />

            <div class="pillars">
                <div class="pillar pillar-purple" data-reveal="" data-reveal-delay="1">
                    <div class="pillar-icon">
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <circle cx="12" cy="6" r="2"/><circle cx="6" cy="18" r="2"/><circle cx="18" cy="18" r="2"/><circle cx="6" cy="12" r="2"/><circle cx="18" cy="12" r="2"/>
                            <line x1="12" y1="8" x2="6" y2="12"/><line x1="12" y1="8" x2="18" y2="12"/><line x1="6" y1="14" x2="6" y2="16"/><line x1="18" y1="14" x2="18" y2="16"/>
                        </svg>
                    </div>
                    <h3>"Squad structure"</h3>
                    <p>"Small teams of 5+5 named after in-game lore. Your squad is your crew \u{2014} the org is the scaffold that holds it together."</p>
                </div>
                <div class="pillar pillar-gold" data-reveal="" data-reveal-delay="2">
                    <div class="pillar-icon">
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <rect x="2" y="4" width="8" height="8" rx="1"/><rect x="14" y="12" width="8" height="8" rx="1"/>
                            <line x1="10" y1="8" x2="14" y2="16"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/>
                        </svg>
                    </div>
                    <h3>"Multi-game"</h3>
                    <p>"Overwatch, Destiny 2, and whatever comes next. The crew spans games \u{2014} your squad plays one, the org plays them all."</p>
                </div>
                <div class="pillar pillar-blue" data-reveal="" data-reveal-delay="3">
                    <div class="pillar-icon">
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <rect x="9" y="2" width="6" height="10" rx="3"/>
                            <path d="M5 10a7 7 0 0 0 14 0"/>
                            <line x1="12" y1="17" x2="12" y2="22"/>
                            <path d="M19 10c1.5-1 2.5-1 3 0"/>
                        </svg>
                    </div>
                    <h3>"Dedicated voice"</h3>
                    <p>"Play nights run on TeamSpeak \u{2014} self-hosted, low latency, built for competitive gaming. Matrix handles everything else."</p>
                </div>
                <div class="pillar pillar-green" data-reveal="" data-reveal-delay="4">
                    <div class="pillar-icon">
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
                            <line x1="12" y1="8" x2="12" y2="12"/><circle cx="12" cy="15" r="0.5" fill="currentColor"/>
                        </svg>
                    </div>
                    <h3>"One rule"</h3>
                    <p>"No politics, no drama, no soapboxes. Show up, communicate, have fun. Ghost for weeks and your slot opens up."</p>
                </div>
            </div>
        </section>
    }
}
