//! The Armature app with the built-in panels only. A build with panels of your own has its own
//! `main` that adds them: `armature::App::new().panel(Mine::new()).run()`.

fn main() -> Result<(), armature::Error> {
    armature::App::new().run()
}
