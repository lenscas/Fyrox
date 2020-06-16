//! File browser is a tree view over file system. It allows to select file or folder.

use std::{
    path::{PathBuf, Path},
    ops::{Deref, DerefMut},
    rc::Rc,
};
use crate::{
    grid::{GridBuilder, Column, Row},
    text_box::TextBoxBuilder,
    text::TextBuilder,
    tree::{TreeBuilder, TreeRootBuilder},
    message::{
        UiMessage,
        UiMessageData,
        FileBrowserMessage,
        TreeRootMessage,
        TextBoxMessage,
        TreeMessage,
        TextMessage
    },
    node::UINode,
    widget::{Widget, WidgetBuilder},
    Control,
    NodeHandleMapping,
    UserInterface,
    core::pool::Handle,
    scroll_viewer::ScrollViewerBuilder,
    Thickness,
    BuildContext,
};
use std::cell::RefCell;

pub type Filter = dyn FnMut(&Path) -> bool;

pub struct FileBrowser<M: 'static, C: 'static + Control<M, C>> {
    widget: Widget<M, C>,
    tree_root: Handle<UINode<M, C>>,
    path: PathBuf,
    path_text: Handle<UINode<M, C>>,
    selection: PathBuf,
    filter: Option<Rc<RefCell<Filter>>>,
}

impl<M: 'static, C: 'static + Control<M, C>> Deref for FileBrowser<M, C> {
    type Target = Widget<M, C>;

    fn deref(&self) -> &Self::Target {
        &self.widget
    }
}

impl<M: 'static, C: 'static + Control<M, C>> DerefMut for FileBrowser<M, C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.widget
    }
}

impl<M: 'static, C: 'static + Control<M, C>> Clone for FileBrowser<M, C> {
    fn clone(&self) -> Self {
        Self {
            widget: self.widget.raw_copy(),
            tree_root: self.tree_root,
            path: self.path.clone(),
            path_text: self.path_text,
            selection: self.selection.clone(),
            filter: self.filter.clone(),
        }
    }
}

impl<M: 'static, C: 'static + Control<M, C>> Control<M, C> for FileBrowser<M, C> {
    fn raw_copy(&self) -> UINode<M, C> {
        UINode::FileBrowser(self.clone())
    }

    fn resolve(&mut self, node_map: &NodeHandleMapping<M, C>) {
        self.tree_root = *node_map.get(&self.tree_root).unwrap();
        self.path_text = *node_map.get(&self.path_text).unwrap();
    }

    fn handle_routed_message(&mut self, ui: &mut UserInterface<M, C>, message: &mut UiMessage<M, C>) {
        self.widget.handle_routed_message(ui, message);

        match &message.data {
            UiMessageData::FileBrowser(msg) => {
                if message.destination == self.handle() {
                    match msg {
                        FileBrowserMessage::Path(path) => {
                            // Rebuild tree.
                            ui.send_message(UiMessage {
                                handled: false,
                                data: UiMessageData::TreeRoot(TreeRootMessage::Items(vec![])),
                                destination: self.tree_root,
                            });
                            build_tree(self.tree_root, true, path, Path::new(""), ui);
                        }
                        FileBrowserMessage::SelectionChanged(path) => {
                            if &self.selection != path {
                                // TODO: Maybe its better to make new tree if new path exists in
                                //  file system?
                                let tree = find_tree(self.tree_root, path, ui);
                                if tree.is_some() {
                                    self.selection = path.clone();
                                    ui.send_message(TextMessage::text(self.path_text, path.to_string_lossy().to_string()));
                                    ui.send_message(TreeRootMessage::select(self.tree_root, tree));
                                }
                            }
                        }
                    }
                }
            }
            UiMessageData::TextBox(msg) => {
                if message.destination == self.path_text {
                    if let TextBoxMessage::Text(txt) = msg {
                        // Try to find tree corresponding to path.
                        let tree = find_tree(self.tree_root, txt, ui);
                        if tree.is_some() {
                            ui.send_message(TreeRootMessage::select(self.tree_root, tree));
                        }
                    }
                }
            }
            UiMessageData::Tree(msg) => {
                if let TreeMessage::Expand(expand) = *msg {
                    if expand {
                        // Look into internals of directory and build tree items.
                        if let UINode::Tree(tree) = ui.node(message.destination) {
                            let parent_path = tree.user_data_ref::<PathBuf>().clone();
                            if let Ok(dir_iter) = std::fs::read_dir(&parent_path) {
                                for p in dir_iter {
                                    if let Ok(entry) = p {
                                        let path = entry.path();
                                        let build = if let Some(filter) = self.filter.as_ref() {
                                            filter.deref().borrow_mut().deref_mut()(&path)
                                        } else {
                                            true
                                        };
                                        if build {
                                            build_tree(message.destination, false, &path, &parent_path, ui);
                                        }
                                    }
                                }
                            }
                        } else {
                            panic!("must be tree");
                        }
                    } else {
                        // Nuke everything in collapsed item. This also will free some resources
                        // and will speed up layout pass.
                        ui.send_message(TreeMessage::set_items(message.destination, vec![]));
                    }
                }
            }
            UiMessageData::TreeRoot(msg) => {
                if message.destination == self.tree_root {
                    if let TreeRootMessage::Selected(selection) = *msg {
                        let path = ui.node(selection).user_data_ref::<PathBuf>();
                        if &self.selection != path {
                            ui.send_message(UiMessage {
                                handled: false,
                                data: UiMessageData::FileBrowser(FileBrowserMessage::SelectionChanged(path.as_path().to_owned())),
                                destination: self.handle(),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn remove_ref(&mut self, handle: Handle<UINode<M, C>>) {
        if self.tree_root == handle {
            self.tree_root = Handle::NONE;
        }
        if self.path_text == handle {
            self.path_text = Handle::NONE;
        }
    }
}

fn find_tree<M: 'static, C: 'static + Control<M, C>, P: AsRef<Path>>(node: Handle<UINode<M, C>>, path: &P, ui: &UserInterface<M, C>) -> Handle<UINode<M, C>> {
    let mut tree_handle = Handle::NONE;
    match ui.node(node) {
        UINode::Tree(tree) => {
            let tree_path = tree.user_data_ref::<PathBuf>();
            if tree_path.to_string_lossy().starts_with(path.as_ref().to_string_lossy().deref()) {
                tree_handle = node;
            }
            for &item in tree.items() {
                let tree = find_tree(item, path, ui);
                if tree.is_some() {
                    tree_handle = tree;
                    break;
                }
            }
        }
        UINode::TreeRoot(root) => {
            for &item in root.items() {
                let tree = find_tree(item, path, ui);
                if tree.is_some() {
                    tree_handle = tree;
                    break;
                }
            }
        }
        _ => unreachable!()
    }
    tree_handle
}

fn build_tree_item<M: 'static, C: 'static + Control<M, C>>(path: &Path, parent_path: &Path, ctx: &mut BuildContext<M, C>) -> Handle<UINode<M, C>> {
    let is_dir_empty = path.read_dir().map_or(true, |mut f| f.next().is_none());
    TreeBuilder::new(WidgetBuilder::new()
        .with_user_data(Rc::new(path.to_owned())))
        .with_expanded(false)
        .with_always_show_expander(!is_dir_empty)
        .with_content(TextBuilder::new(WidgetBuilder::new())
            .with_text(path.to_string_lossy().replace(&parent_path.to_string_lossy().to_string(), ""))
            .build(ctx))
        .build(ctx)
}

fn build_tree<M: 'static, C: 'static + Control<M, C>>(parent: Handle<UINode<M, C>>, is_parent_root: bool, path: &Path, parent_path: &Path, ui: &mut UserInterface<M, C>) -> Handle<UINode<M, C>> {
    let tree = build_tree_item(path, parent_path, &mut ui.build_ctx());

    if is_parent_root {
        ui.send_message(TreeRootMessage::add_item(parent, tree));
    } else {
        ui.send_message(TreeMessage::add_item(parent, tree));
    }

    tree
}

pub struct FileBrowserBuilder<M: 'static, C: 'static + Control<M, C>> {
    widget_builder: WidgetBuilder<M, C>,
    path: PathBuf,
    filter: Option<Rc<RefCell<Filter>>>,
}

impl<M: 'static, C: 'static + Control<M, C>> FileBrowserBuilder<M, C> {
    pub fn new(widget_builder: WidgetBuilder<M, C>) -> Self {
        Self {
            widget_builder,
            path: Default::default(),
            filter: None,
        }
    }

    pub fn with_filter(mut self, filter: Rc<RefCell<Filter>>) -> Self {
        self.filter = Some(filter);
        self
    }

    pub fn with_path<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.path = path.as_ref().to_owned();
        self
    }

    pub fn build(self, ctx: &mut BuildContext<M, C>) -> Handle<UINode<M, C>> {
        let path_text;
        let tree_root;
        let scroll_viewer = ScrollViewerBuilder::new(WidgetBuilder::new()
            .on_row(1)
            .on_column(0))
            .with_content({
                tree_root = TreeRootBuilder::new(WidgetBuilder::new())
                    .with_items(vec![build_tree_item(&self.path, Path::new(""), ctx)])
                    .build(ctx);
                tree_root
            })
            .build(ctx);

        let grid = GridBuilder::new(WidgetBuilder::new()
            .with_child({
                path_text = TextBoxBuilder::new(WidgetBuilder::new()
                    .on_row(0)
                    .on_column(0)
                    .with_margin(Thickness::uniform(1.0)))
                    .with_text("Foobar")
                    .build(ctx);
                path_text
            })
            .with_child(scroll_viewer))
            .add_column(Column::auto())
            .add_row(Row::strict(30.0))
            .add_row(Row::stretch())
            .build(ctx);

        let browser = FileBrowser {
            widget: self.widget_builder
                .with_child(grid)
                .build(),
            tree_root,
            path: self.path,
            path_text,
            selection: Default::default(),
            filter: self.filter,
        };

        ctx.add_node(UINode::FileBrowser(browser))
    }
}