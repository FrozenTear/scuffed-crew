use leptos::prelude::*;

#[component]
pub fn Recruit() -> impl IntoView {
    view! {
        <section id="join">
            <div class="recruit-wrap" data-reveal="">
                <div class="recruit-left">
                    <div class="sec-label sec-label-purple">"// Recruitment Open"</div>
                    <h2>"Want in?"</h2>
                    <p>"We keep rosters intentional. Join our Matrix server, hop in a few play nights, and we\u{2019}ll match you with a team that fits your schedule and skill level. Play 3\u{2013}5 sessions with us. If we\u{2019}re having fun together, you\u{2019}re in."</p>
                    <div style="display:flex;gap:1rem;flex-wrap:wrap;">
                        <a href="#" class="btn btn-primary">"Join Matrix"</a>
                    </div>
                    <div class="recruit-seeking">
                        <div class="recruit-seeking-label">"Currently looking for"</div>
                        <div class="recruit-seeking-tags">
                            <span class="recruit-tag recruit-tag-ow">"OW2 DPS"</span>
                            <span class="recruit-tag recruit-tag-ow">"OW2 Support"</span>
                            <span class="recruit-tag recruit-tag-dest">"D2 PvP"</span>
                        </div>
                    </div>
                </div>
                <div class="recruit-right">
                    <h3>"What we expect"</h3>
                    <div class="req"><span class="req-marker">"\u{203A}"</span><span>"16+ \u{2014} old enough to communicate and commit to a schedule"</span></div>
                    <div class="req"><span class="req-marker">"\u{203A}"</span><span>"PC only \u{2014} our teams play on PC"</span></div>
                    <div class="req"><span class="req-marker">"\u{203A}"</span><span>"Communicate \u{2014} let your squad know if you can\u{2019}t make it"</span></div>
                    <div class="req"><span class="req-marker">"\u{203A}"</span><span>"No toxicity \u{2014} competitive is fine, being a jerk isn\u{2019}t"</span></div>
                    <div class="req"><span class="req-marker">"\u{203A}"</span><span>"Mic required for play nights"</span></div>
                    <div class="req"><span class="req-marker">"\u{203A}"</span><span>"Willing to install TeamSpeak when you make a roster"</span></div>

                    <div class="never-ask">
                        <div class="never-ask-header">"What we\u{2019}ll never ask for"</div>
                        <div class="never-ask-body">
                            "Your real name \u{00B7} Your email \u{00B7} Your phone number \u{00B7} Your social media \u{00B7} Your location \u{00B7} Your government ID \u{00B7} Access to your contacts \u{00B7} Permission to scan your processes"
                        </div>
                    </div>
                </div>
            </div>
        </section>
    }
}
