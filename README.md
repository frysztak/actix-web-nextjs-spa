# Actix Web Next.js SPA service

Actix Web service for hosting [statically exported](https://nextjs.org/docs/app/building-your-application/deploying/static-exports) Next.js apps.

This is a fork of [Spa service from actix-web-lab](https://docs.rs/actix-web-lab/0.19.1/actix_web_lab/web/fn.spa.html) with added support for Next.js dynamic routes.

## How it works

It searches for Next.js's `_buildManifest.js` and builds a tree of routes from it. Request to, e.g., `/pet/dog/husky` resolves into `/pet/[petType]/[breed].html`.

## Sample usage

Exactly the same as original SPA service:

```rust
use actix_web::App;
use actix_web_nextjs_spa::spa;
let app = App::new()
    // ...api routes...
    .service(
        spa()
            .index_file("./web/spa.html")
            .static_resources_location("./web")
            .finish()
    );
```

## License

This project is licensed under either of the following licenses, at your option:

- Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0])
- MIT license ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT])