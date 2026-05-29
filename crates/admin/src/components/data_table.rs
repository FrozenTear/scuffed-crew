use leptos::prelude::*;

/// A simple data table component.
#[component]
pub fn DataTable(headers: Vec<&'static str>, children: Children) -> impl IntoView {
    view! {
        <table class="data-table">
            <thead>
                <tr>
                    {headers.into_iter().map(|h| view! { <th>{h}</th> }).collect_view()}
                </tr>
            </thead>
            <tbody>
                {children()}
            </tbody>
        </table>
    }
}
