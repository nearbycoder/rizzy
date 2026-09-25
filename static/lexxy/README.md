# Lexxy assets

These are the JavaScript and stylesheets from the `lexxy` Ruby gem at version
`0.9.32`, the version pinned by the reference Fizzy revision. They are served
by Rizzy's Rust HTTP server so its rich text editors use the same browser
component as Fizzy. The upstream gem's MIT license is included in
`MIT-LICENSE`.

The JavaScript file is the gem's `lexxy.min.js`, renamed to `lexxy.js` for the
Rizzy asset route. `rizzy-dark.css` is Rizzy's own theme override.

Lexxy loads `@rails/activestorage` when a user uploads a file. The
`activestorage.esm.js` file is from the Rails revision in Fizzy's bundle
(`3df2cbea2027`) and is mapped by the editor pages' import maps. Its MIT
license is included in `RAILS-MIT-LICENSE`.
