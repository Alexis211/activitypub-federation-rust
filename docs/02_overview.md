## Overview

It is recommended to read the [W3C Activitypub standard document](https://www.w3.org/TR/activitypub/) which explains in detail how the protocol works. Note that it includes a section about client to server interactions, this functionality is not implemented by any major Fediverse project. Other relevant standard documents are [Activitystreams](https://www.w3.org/ns/activitystreams) and [Activity Vocabulary](https://www.w3.org/TR/activitystreams-vocabulary/). Its a good idea to keep these around as references during development.

This crate provides high level abstractions for the core functionality of Activitypub: fetching, sending and receiving data, as well as handling HTTP signatures. It was built from the experience of developing [Lemmy](https://join-lemmy.org/) which is the biggest Fediverse project written in Rust. Nevertheless it very generic and appropriate for any type of application wishing to implement the Activitypub protocol.

There are two examples included to see how the library altogether:

- `local_federation`: Creates two instances which run on localhost and federate with each other. This setup is ideal for quick development and well as automated tests.
- `live_federation`: A minimal application which can be deployed on a server and federate with other platforms such as Mastodon. For this it needs run at the root of a (sub)domain which is available over HTTPS. Edit `main.rs` to configure the server domain and your Fediverse handle. Once started, it will automatically send a message to you and log any incoming messages.

To see how this library is used in production, have a look at the [Lemmy federation code](https://github.com/LemmyNet/lemmy/tree/main/crates/apub).
