fn main() {
    // exe 图标（资源管理器、任务栏、托盘都从这里取，资源序号 1）
    embed_resource::compile("assets/app.rc", embed_resource::NONE).manifest_optional().unwrap();
    println!("cargo:rerun-if-changed=assets/app.ico");
}
