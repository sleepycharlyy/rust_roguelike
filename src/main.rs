use rand::Rng;
use std::cmp;
use tcod::colors::*;
use tcod::console::*;
use tcod::input::{self, Event, Key, Mouse};
use tcod::map::{FovAlgorithm, Map as FovMap};

// actual size of the window
const SCREEN_WIDTH: i32 = 80;
const SCREEN_HEIGHT: i32 = 50;

// size of the map
const MAP_WIDTH: i32 = 80;
const MAP_HEIGHT: i32 = 40;

// parameters for dungeon generator
const ROOM_MAX_SIZE: i32 = 10;
const ROOM_MIN_SIZE: i32 = 6;
const MAX_ROOMS: i32 = 30;
const MAX_ROOM_MONSTERS: i32 = 3;
const MAX_ROOM_ITEMS: i32 = 2;

// item parameters
const HEAL_AMOUNT: i32 = 4;
const LIGHTNING_DAMAGE: i32 = 40;
const LIGHTNING_RANGE: i32 = 5;
const CONFUSE_RANGE: i32 = 8;
const CONFUSE_NUM_TURNS: i32 = 10;

// message log
const MSG_X: i32 = BAR_WIDTH + 2;
const MSG_WIDTH: i32 = SCREEN_WIDTH - BAR_WIDTH - 2;
const MSG_HEIGHT: usize = PANEL_HEIGHT as usize - 1;

// fov
const FOV_ALGO: FovAlgorithm = FovAlgorithm::Basic; // default FOV algorithm
const FOV_LIGHT_WALLS: bool = true; // light walls or not
const TORCH_RADIUS: i32 = 10;

// sizes and coordinates relevant for the GUI
const BAR_WIDTH: i32 = 20;
const PANEL_HEIGHT: i32 = 10;
const PANEL_Y: i32 = SCREEN_HEIGHT - PANEL_HEIGHT;
const INVENTORY_WIDTH: i32 = 50;

const LIMIT_FPS: i32 = 20; // 20 frames-per-second maximum

const COLOR_DARK_WALL: Color = Color { r: 0, g: 0, b: 100 };
const COLOR_LIGHT_WALL: Color = Color {
    r: 130,
    g: 110,
    b: 50,
};
const COLOR_DARK_GROUND: Color = Color {
    r: 50,
    g: 50,
    b: 150,
};
const COLOR_LIGHT_GROUND: Color = Color {
    r: 200,
    g: 180,
    b: 50,
};

// player will always be the first object
const PLAYER: usize = 0;

struct Tcod {
    root: Root,
    console: Offscreen,
    panel: Offscreen,
    fov: FovMap,
    key: Key,
    mouse: Mouse,
}

type Map = Vec<Vec<Tile>>;

struct Game {
    map: Map,
    messages: Messages,
    inventory: Vec<Object>,
}

/// message log message struct
struct Messages {
    messages: Vec<(String, Color)>,
}

impl Messages {
    pub fn new() -> Self {
        Self { messages: vec![] }
    }

    /// add the new message as a tuple, with the text and the color
    pub fn add<T: Into<String>>(&mut self, message: T, color: Color) {
        self.messages.push((message.into(), color));
    }

    /// create a `` over the messages
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &(String, Color)> {
        self.messages.iter()
    }
}

/// a tile of the map and its properties
#[derive(Clone, Copy, Debug)]
struct Tile {
    blocked: bool,
    explored: bool,
    block_sight: bool,
}

impl Tile {
    pub fn empty() -> Self {
        Tile {
            blocked: false,
            explored: false,
            block_sight: false,
        }
    }

    pub fn wall() -> Self {
        Tile {
            blocked: true,
            explored: false,
            block_sight: true,
        }
    }
}

/// a rectangle on the map, used to characterise a room.
#[derive(Clone, Copy, Debug)]
struct Room {
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
}

impl Room {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Room {
            x1: x,
            y1: y,
            x2: x + w,
            y2: y + h,
        }
    }

    pub fn center(&self) -> (i32, i32) {
        let center_x = (self.x1 + self.x2) / 2;
        let center_y = (self.y1 + self.y2) / 2;
        (center_x, center_y)
    }

    pub fn intersects_with(&self, other: &Room) -> bool {
        // returns true if this rectangle intersects with another one
        (self.x1 <= other.x2)
            && (self.x2 >= other.x1)
            && (self.y1 <= other.y2)
            && (self.y2 >= other.y1)
    }
}

/// this is a generic object: the player, a monster, an item, the stairs...
/// it's always represented by a character on screen.
#[derive(Debug)]
struct Object {
    x: i32,
    y: i32,
    char: char,
    color: Color,
    name: String,
    blocks: bool,
    alive: bool,
    fighter: Option<Fighter>,
    ai: Option<Ai>,
    item: Option<Item>,
}

impl Object {
    pub fn new(x: i32, y: i32, char: char, name: &str, color: Color, blocks: bool) -> Self {
        Object {
            x: x,
            y: y,
            char: char,
            color: color,
            name: name.into(),
            blocks: blocks,
            alive: false,
            fighter: None,
            ai: None,
            item: None,
        }
    }

    /// set the color and then draw the character that represents this object at its position
    pub fn draw(&self, console: &mut dyn Console) {
        console.set_default_foreground(self.color);
        console.put_char(self.x, self.y, self.char, BackgroundFlag::None);
    }

    /// get position
    pub fn pos(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    /// set position
    pub fn set_pos(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
    }

    /// return the distance to another object
    pub fn distance_to(&self, other: &Object) -> f32 {
        let x_amount = other.x - self.x;
        let y_amount = other.y - self.y;
        ((x_amount.pow(2) + y_amount.pow(2)) as f32).sqrt()
    }

    /// take damage
    pub fn take_damage(&mut self, damage: i32, game: &mut Game) {
        // apply damage if possible
        if let Some(fighter) = self.fighter.as_mut() {
            if damage > 0 {
                fighter.hp -= damage;
            }
        }
        // check for death, call the death function
        if let Some(fighter) = self.fighter {
            if fighter.hp <= 0 {
                self.alive = false;
                fighter.on_death.callback(self, game);
            }
        }
    }

    /// heal by the given amount, without going over the maximum
    pub fn heal(&mut self, amount: i32) {
        if let Some(ref mut fighter) = self.fighter {
            fighter.hp += amount;
            if fighter.hp > fighter.max_hp {
                fighter.hp = fighter.max_hp;
            }
        }
    }

    /// attack target fighter
    pub fn attack(&mut self, target: &mut Object, game: &mut Game) {
        // a simple formula for attack damage
        let damage = self.fighter.map_or(0, |f| f.power) - target.fighter.map_or(0, |f| f.defense);
        if damage > 0 {
            // make the target take some damage
            game.messages.add(
                format!(
                    "{} attacks {} for {} hit points.",
                    self.name, target.name, damage
                ),
                WHITE,
            );
            target.take_damage(damage, game);
        } else {
            game.messages.add(
                format!(
                    "{} attacks {} but it has no effect!",
                    self.name, target.name
                ),
                WHITE,
            );
        }
    }
}

/// create popup menu window
fn menu<T: AsRef<str>>(header: &str, options: &[T], width: i32, root: &mut Root) -> Option<usize> {
    assert!(
        options.len() <= 26,
        "Cannot have a menu with more than 26 options."
    );

    // calculate total height for the header (after auto-wrap) and one line per option
    let header_height = root.get_height_rect(0, 0, width, SCREEN_HEIGHT, header);
    let height = options.len() as i32 + header_height;

    // create an off-screen console that represents the menu's window
    let mut window = Offscreen::new(width, height);

    // print the header, with auto-wrap
    window.set_default_foreground(WHITE);
    window.print_rect_ex(
        0,
        0,
        width,
        height,
        BackgroundFlag::None,
        TextAlignment::Left,
        header,
    );

    // print all the options
    for (index, option_text) in options.iter().enumerate() {
        let menu_letter = (b'a' + index as u8) as char;
        let text = format!("({}) {}", menu_letter, option_text.as_ref());
        window.print_ex(
            0,
            header_height + index as i32,
            BackgroundFlag::None,
            TextAlignment::Left,
            text,
        );
    }

    // blit the contents of "window" to the root console
    let x = SCREEN_WIDTH / 2 - width / 2;
    let y = SCREEN_HEIGHT / 2 - height / 2;
    blit(&window, (0, 0), (width, height), root, (x, y), 1.0, 0.7);

    // present the root console to the player and wait for a key-press
    root.flush();
    let key = root.wait_for_keypress(true);

    // convert the ASCII code to an index; if it corresponds to an option, return it
    if key.printable.is_alphabetic() {
        let index = key.printable.to_ascii_lowercase() as usize - 'a' as usize;
        if index < options.len() {
            Some(index)
        } else {
            None
        }
    } else {
        None
    }
}

/// create an inventory menu
fn menu_inventory(inventory: &[Object], header: &str, root: &mut Root) -> Option<usize> {
    // how a menu with each item of the inventory as an option
    let options = if inventory.len() == 0 {
        vec!["Inventory is empty.".into()]
    } else {
        inventory.iter().map(|item| item.name.clone()).collect()
    };

    let inventory_index = menu(header, &options, INVENTORY_WIDTH, root);

    // if an item was chosen, return it
    if inventory.len() > 0 {
        inventory_index
    } else {
        None
    }
}

/// move by the given amount, if the destination is not blocked
fn object_move_by(id: usize, x_amount: i32, y_amount: i32, map: &Map, objects: &mut [Object]) {
    let (x, y) = objects[id].pos();
    if !tile_is_blocked(x + x_amount, y + y_amount, map, objects) {
        objects[id].set_pos(x + x_amount, y + y_amount);
    }
}

fn object_move_towards(id: usize, target_x: i32, target_y: i32, map: &Map, objects: &mut [Object]) {
    // vector from this object to the target, and distance
    let x_amount = target_x - objects[id].x;
    let y_amount = target_y - objects[id].y;
    let distance = ((x_amount.pow(2) + y_amount.pow(2)) as f32).sqrt();

    // normalize it to length 1 (preserving direction), then round it and
    // convert to integer so the movement is restricted to the map grid
    let x_amount = (x_amount as f32 / distance).round() as i32;
    let y_amount = (y_amount as f32 / distance).round() as i32;
    object_move_by(id, x_amount, y_amount, map, objects);
}

fn tile_is_blocked(x: i32, y: i32, map: &Map, objects: &[Object]) -> bool {
    // first test the map tile
    if map[x as usize][y as usize].blocked {
        return true;
    }
    // now check for any blocking objects
    objects
        .iter()
        .any(|object| object.blocks && object.pos() == (x, y))
}

// combat-related properties and methods (monster, player, NPC).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fighter {
    max_hp: i32,
    hp: i32,
    defense: i32,
    power: i32,
    on_death: DeathCallback,
}

/// ai type
#[derive(Clone, Debug, PartialEq)]
enum Ai {
    Basic,
    Confused {
        previous_ai: Box<Ai>,
        num_turns: i32,
    },
}

/// let ai take turn
fn ai_take_turn(monster_id: usize, tcod: &Tcod, game: &mut Game, objects: &mut [Object]) {
    use Ai::*;
    if let Some(ai) = objects[monster_id].ai.take() {
        let new_ai = match ai {
            Basic => ai_basic(monster_id, tcod, game, objects),
            Confused {
                previous_ai,
                num_turns,
            } => ai_confused(monster_id, tcod, game, objects, previous_ai, num_turns),
        };
        objects[monster_id].ai = Some(new_ai);
    }
}

/// basic ai type (follows player if in range)
fn ai_basic(monster_id: usize, tcod: &Tcod, game: &mut Game, objects: &mut [Object]) -> Ai {
    // a basic monster takes its turn. If you can see it, it can see you
    let (monster_x, monster_y) = objects[monster_id].pos();
    if tcod.fov.is_in_fov(monster_x, monster_y) {
        if objects[monster_id].distance_to(&objects[PLAYER]) >= 2.0 {
            // move towards player if far away
            let (player_x, player_y) = objects[PLAYER].pos();
            object_move_towards(monster_id, player_x, player_y, &game.map, objects);
        } else if objects[PLAYER].fighter.map_or(false, |f| f.hp > 0) {
            // close enough, attack! (if the player is still alive.)
            let (monster, player) = mut_two(monster_id, PLAYER, objects);
            monster.attack(player, game);
        }
    }
    Ai::Basic
}

// confused ai type (wanders randomly for a few seconds)
fn ai_confused(
    monster_id: usize,
    _tcod: &Tcod,
    game: &mut Game,
    objects: &mut [Object],
    previous_ai: Box<Ai>,
    num_turns: i32,
) -> Ai {
    if num_turns >= 0 {
        // still confused ...
        // move in a random direction, and decrease the number of turns confused
        object_move_by(
            monster_id,
            rand::thread_rng().gen_range(-1, 2),
            rand::thread_rng().gen_range(-1, 2),
            &game.map,
            objects,
        );
        Ai::Confused {
            previous_ai: previous_ai,
            num_turns: num_turns - 1,
        }
    } else {
        // restore the previous AI (this one will be deleted)
        game.messages.add(
            format!("The {} is no longer confused!", objects[monster_id].name),
            RED,
        );
        *previous_ai
    }
}

fn create_room(room: Room, map: &mut Map) {
    // go through the tiles in the rectangle and make them passable
    for x in (room.x1 + 1)..room.x2 {
        for y in (room.y1 + 1)..room.y2 {
            map[x as usize][y as usize] = Tile::empty();
        }
    }
}

fn create_h_tunnel(x1: i32, x2: i32, y: i32, map: &mut Map) {
    // horizontal tunnel. `min()` and `max()` are used in case `x1 > x2`
    for x in cmp::min(x1, x2)..(cmp::max(x1, x2) + 1) {
        map[x as usize][y as usize] = Tile::empty();
    }
}

fn create_v_tunnel(y1: i32, y2: i32, x: i32, map: &mut Map) {
    // vertical tunnel
    for y in cmp::min(y1, y2)..(cmp::max(y1, y2) + 1) {
        map[x as usize][y as usize] = Tile::empty();
    }
}

fn create_map(objects: &mut Vec<Object>) -> Map {
    // fill map with "blocked" tiles
    let mut map = vec![vec![Tile::wall(); MAP_HEIGHT as usize]; MAP_WIDTH as usize];

    let mut rooms = vec![];

    for _ in 0..MAX_ROOMS {
        // random width and height
        let w = rand::thread_rng().gen_range(ROOM_MIN_SIZE, ROOM_MAX_SIZE + 1);
        let h = rand::thread_rng().gen_range(ROOM_MIN_SIZE, ROOM_MAX_SIZE + 1);
        // random position without going out of the boundaries of the map
        let x = rand::thread_rng().gen_range(0, MAP_WIDTH - w);
        let y = rand::thread_rng().gen_range(0, MAP_HEIGHT - h);

        let new_room = Room::new(x, y, w, h);

        // run through the other rooms and see if they intersect with this one
        let failed = rooms
            .iter()
            .any(|other_room| new_room.intersects_with(other_room));

        if !failed {
            // this means there are no intersections, so this room is valid

            // carve room out of the map's tiles
            create_room(new_room, &mut map);

            // add some content to this room, such as monsters
            populate_room(new_room, &map, objects);

            // center coordinates of the new room, will be useful later
            let (new_x, new_y) = new_room.center();

            if rooms.is_empty() {
                // this is the first room, where the player starts at
                objects[PLAYER].set_pos(new_x, new_y);
            } else {
                // all rooms after the first:
                // connect it to the previous room with a tunnel

                // center coordinates of the previous room
                let (prev_x, prev_y) = rooms[rooms.len() - 1].center();

                // toss a coin (random bool value -- either true or false)
                if rand::random() {
                    // first move horizontally, then vertically
                    create_h_tunnel(prev_x, new_x, prev_y, &mut map);
                    create_v_tunnel(prev_y, new_y, new_x, &mut map);
                } else {
                    // first move vertically, then horizontally
                    create_v_tunnel(prev_y, new_y, prev_x, &mut map);
                    create_h_tunnel(prev_x, new_x, new_y, &mut map);
                }
            }

            // finally, append the new room to the list
            rooms.push(new_room);
        }
    }

    map
}

/// use an item
fn use_item(inventory_id: usize, tcod: &mut Tcod, game: &mut Game, objects: &mut [Object]) {
    use Item::*;
    // just call the "use_function" if it is defined
    if let Some(item) = game.inventory[inventory_id].item {
        let on_use = match item {
            Heal => cast_heal,
            Lightning => cast_lightning,
            Confuse => cast_confuse,
        };
        match on_use(inventory_id, tcod, game, objects) {
            UseResult::UsedUp => {
                // destroy after use, unless it was cancelled for some reason
                game.inventory.remove(inventory_id);
            }
            UseResult::Cancelled => {
                game.messages.add("Cancelled", WHITE);
            }
        }
    } else {
        game.messages.add(
            format!("The {} cannot be used.", game.inventory[inventory_id].name),
            WHITE,
        );
    }
}

/// return a string with the names of all objects under the mouse
fn get_names_under_mouse(mouse: Mouse, objects: &[Object], fov_map: &FovMap) -> String {
    let (x, y) = (mouse.cx as i32, mouse.cy as i32);

    // create a list with the names of all objects at the mouse's coordinates and in FOV
    let names = objects
        .iter()
        .filter(|obj| obj.pos() == (x, y) && fov_map.is_in_fov(obj.x, obj.y))
        .map(|obj| obj.name.clone())
        .collect::<Vec<_>>();

    names.join(", ") // join the names, separated by commas
}

fn populate_room(room: Room, map: &Map, objects: &mut Vec<Object>) {
    // Items
    // choose random number of items
    let num_items = rand::thread_rng().gen_range(0, MAX_ROOM_ITEMS + 1);

    for _ in 0..num_items {
        // choose random spot for this item
        let x = rand::thread_rng().gen_range(room.x1 + 1, room.x2);
        let y = rand::thread_rng().gen_range(room.y1 + 1, room.y2);

        // only place it if the tile is not blocked
        if !tile_is_blocked(x, y, map, objects) {
            let dice = rand::random::<f32>();
            let item = if dice < 0.7 {
                // create a healing potion (70% chance)
                let mut object = Object::new(x, y, '!', "healing potion", VIOLET, false);
                object.item = Some(Item::Heal);
                object
            } else if dice < 0.7 + 0.1 {
                // create a lightning bolt scroll (10% chance)
                let mut object =
                    Object::new(x, y, '#', "scroll of lightning bolt", LIGHT_YELLOW, false);
                object.item = Some(Item::Lightning);
                object
            } else {
                // create a confuse scroll (10% chance)
                let mut object = Object::new(x, y, '#', "scroll of confusion", LIGHT_YELLOW, false);
                object.item = Some(Item::Confuse);
                object
            };
            objects.push(item);
        }
    }

    // Monsters
    // choose random number of monsters
    let num_monsters = rand::thread_rng().gen_range(0, MAX_ROOM_MONSTERS + 1);

    for _ in 0..num_monsters {
        // choose random spot for this monster
        let x = rand::thread_rng().gen_range(room.x1 + 1, room.x2);
        let y = rand::thread_rng().gen_range(room.y1 + 1, room.y2);

        // only place it if the tile is not blocked
        if !tile_is_blocked(x, y, map, objects) {
            let mut monster = if rand::random::<f32>() < 0.8 {
                // 80% chance of getting an orc
                // create an orc
                let mut orc = Object::new(x, y, 'o', "orc", DESATURATED_GREEN, true);
                orc.fighter = Some(Fighter {
                    max_hp: 10,
                    hp: 10,
                    defense: 0,
                    power: 3,
                    on_death: DeathCallback::Monster,
                });
                orc.ai = Some(Ai::Basic);
                orc
            } else {
                // create a troll
                let mut troll = Object::new(x, y, 'T', "troll", DARKER_GREEN, true);
                troll.fighter = Some(Fighter {
                    max_hp: 16,
                    hp: 16,
                    defense: 1,
                    power: 4,
                    on_death: DeathCallback::Monster,
                });
                troll.ai = Some(Ai::Basic);
                troll
            };
            monster.alive = true;
            objects.push(monster);
        }
    }
}

fn render_all(tcod: &mut Tcod, game: &mut Game, objects: &[Object], fov_recompute: bool) {
    // FOV (Fog of War)
    if fov_recompute {
        // recompute FOV if needed (the player moved or something)
        let player = &objects[PLAYER];
        tcod.fov
            .compute_fov(player.x, player.y, TORCH_RADIUS, FOV_LIGHT_WALLS, FOV_ALGO);
    }

    // Tiles
    // go through all tiles, and set their background color
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            let visible = tcod.fov.is_in_fov(x, y);
            let wall = game.map[x as usize][y as usize].block_sight;
            let color = match (visible, wall) {
                // outside of field of view:
                (false, true) => COLOR_DARK_WALL,
                (false, false) => COLOR_DARK_GROUND,
                // inside fov:
                (true, true) => COLOR_LIGHT_WALL,
                (true, false) => COLOR_LIGHT_GROUND,
            };

            let explored = &mut game.map[x as usize][y as usize].explored;
            if visible {
                // since it's visible, explore it
                *explored = true;
            }
            if *explored {
                // show explored tiles only (any visible tile is explored alreay_amount)
                tcod.console
                    .set_char_background(x, y, color, BackgroundFlag::Set);
            }
        }
    }

    // Objects
    // draw all objects in the list
    for object in objects {
        if tcod.fov.is_in_fov(object.x, object.y) {
            object.draw(&mut tcod.console);
        }
    }

    let mut to_draw: Vec<_> = objects
        .iter()
        .filter(|o| tcod.fov.is_in_fov(o.x, o.y))
        .collect();
    // sort so that non-blocknig objects come first
    to_draw.sort_by(|o1, o2| o1.blocks.cmp(&o2.blocks));
    // draw the objects in the list
    for object in &to_draw {
        object.draw(&mut tcod.console);
    }

    // blit the contents of "console" to the root console
    blit(
        &tcod.console,
        (0, 0),
        (MAP_WIDTH, MAP_HEIGHT),
        &mut tcod.root,
        (0, 0),
        1.0,
        1.0,
    );

    // GUI
    // prepare to render the GUI panel
    tcod.panel.set_default_background(BLACK);
    tcod.panel.clear();

    // print the game messages, one line at a time
    let mut y = MSG_HEIGHT as i32;
    for &(ref msg, color) in game.messages.iter().rev() {
        let msg_height = tcod.panel.get_height_rect(MSG_X, y, MSG_WIDTH, 0, msg);
        y -= msg_height;
        if y < 0 {
            break;
        }
        tcod.panel.set_default_foreground(color);
        tcod.panel.print_rect(MSG_X, y, MSG_WIDTH, 0, msg);
    }

    // show the player's stats
    let hp = objects[PLAYER].fighter.map_or(0, |f| f.hp);
    let max_hp = objects[PLAYER].fighter.map_or(0, |f| f.max_hp);
    render_bar(
        &mut tcod.panel,
        1,
        1,
        BAR_WIDTH,
        "HP",
        hp,
        max_hp,
        LIGHT_RED,
        DARKER_RED,
    );

    // display names of objects under the mouse
    tcod.panel.set_default_foreground(LIGHT_GREY);
    tcod.panel.print_ex(
        1,
        0,
        BackgroundFlag::None,
        TextAlignment::Left,
        get_names_under_mouse(tcod.mouse, objects, &tcod.fov),
    );

    // blit the contents of `panel` to the root console
    blit(
        &tcod.panel,
        (0, 0),
        (SCREEN_WIDTH, PANEL_HEIGHT),
        &mut tcod.root,
        (0, PANEL_Y),
        1.0,
        1.0,
    );
}

fn player_move_or_attack(x_amount: i32, y_amount: i32, game: &mut Game, objects: &mut [Object]) {
    // the coordinates the player is moving to/attacking
    let x = objects[PLAYER].x + x_amount;
    let y = objects[PLAYER].y + y_amount;

    // try to find an attackable object there
    let target_id = objects
        .iter()
        .position(|object| object.fighter.is_some() && object.pos() == (x, y));

    // attack if target found, move otherwise
    match target_id {
        Some(target_id) => {
            // attack
            let (player, target) = mut_two(PLAYER, target_id, objects);
            player.attack(target, game);
        }
        None => {
            object_move_by(PLAYER, x_amount, y_amount, &game.map, objects);
        }
    }
}

fn handle_keys(tcod: &mut Tcod, game: &mut Game, objects: &mut Vec<Object>) -> PlayerAction {
    use tcod::input::KeyCode::*;
    use PlayerAction::*;
    let player_alive = objects[PLAYER].alive;
    match (tcod.key, tcod.key.text(), player_alive) {
        (
            Key {
                code: Enter,
                alt: true,
                ..
            },
            _,
            _,
        ) => {
            // Alt+Enter: toggle fullscreen
            let fullscreen = tcod.root.is_fullscreen();
            tcod.root.set_fullscreen(!fullscreen);
            DidntTakeTurn
        }
        (Key { code: Escape, .. }, _, _) => Exit, // exit game

        // movement keys
        (Key { code: Up, .. }, _, true) => {
            player_move_or_attack(0, -1, game, objects);
            TookTurn
        }
        (Key { code: Down, .. }, _, true) => {
            player_move_or_attack(0, 1, game, objects);
            TookTurn
        }
        (Key { code: Left, .. }, _, true) => {
            player_move_or_attack(-1, 0, game, objects);
            TookTurn
        }
        (Key { code: Right, .. }, _, true) => {
            player_move_or_attack(1, 0, game, objects);
            TookTurn
        }
        (Key { code: Text, .. }, "e", true) => {
            // pick up an item
            let item_id = objects
                .iter()
                .position(|object| object.pos() == objects[PLAYER].pos() && object.item.is_some());
            if let Some(item_id) = item_id {
                pick_item_up(item_id, game, objects);
            }
            DidntTakeTurn
        }
        (Key { code: Text, .. }, "i", true) => {
            // show the inventory: if an item is selected, use it
            let inventory_index = menu_inventory(
                &game.inventory,
                "Press the key next to an item to use it, or any other to cancel.\n",
                &mut tcod.root,
            );
            if let Some(inventory_index) = inventory_index {
                use_item(inventory_index, tcod, game, objects);
            }
            DidntTakeTurn
        }

        _ => DidntTakeTurn,
    }
}

/// Mutably borrow two *separate* elements from the given slice.
/// Panics when the indexes are equal or out of bounds.
fn mut_two<T>(first_index: usize, second_index: usize, items: &mut [T]) -> (&mut T, &mut T) {
    assert!(first_index != second_index);
    let split_at_index = cmp::max(first_index, second_index);
    let (first_slice, second_slice) = items.split_at_mut(split_at_index);
    if first_index < second_index {
        (&mut first_slice[first_index], &mut second_slice[0])
    } else {
        (&mut second_slice[0], &mut first_slice[second_index])
    }
}

fn cast_heal(
    _inventory_id: usize,
    _tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut [Object],
) -> UseResult {
    // heal the player
    if let Some(fighter) = objects[PLAYER].fighter {
        if fighter.hp == fighter.max_hp {
            game.messages.add("You are already at full health.", RED);
            return UseResult::Cancelled;
        }
        game.messages
            .add("Your wounds start to feel better!", LIGHT_VIOLET);
        objects[PLAYER].heal(HEAL_AMOUNT);
        return UseResult::UsedUp;
    }
    UseResult::Cancelled
}

/// cast confusion spell
fn cast_confuse(
    _inventory_id: usize,
    tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut [Object],
) -> UseResult {
    // find closest enemy in-range and confuse it
    let monster_id = target_monster(CONFUSE_RANGE, objects, tcod);
    if let Some(monster_id) = monster_id {
        let old_ai = objects[monster_id].ai.take().unwrap_or(Ai::Basic);
        // replace the monster's AI with a "confused" one; after
        // some turns it will restore the old AI
        objects[monster_id].ai = Some(Ai::Confused {
            previous_ai: Box::new(old_ai),
            num_turns: CONFUSE_NUM_TURNS,
        });
        game.messages.add(
            format!(
                "The eyes of {} look vacant, as he starts to stumble around!",
                objects[monster_id].name
            ),
            LIGHT_GREEN,
        );
        UseResult::UsedUp
    } else {
        // no enemy fonud within maximum range
        game.messages
            .add("No enemy is close enough to strike.", RED);
        UseResult::Cancelled
    }
}

/// cast lightning spell
fn cast_lightning(
    _inventory_id: usize,
    tcod: &mut Tcod,
    game: &mut Game,
    objects: &mut [Object],
) -> UseResult {
    // find closest enemy (inside a maximum range and damage it)
    let monster_id = closest_monster(tcod, objects, LIGHTNING_RANGE);
    if let Some(monster_id) = monster_id {
        // zap it!
        game.messages.add(
            format!(
                "A lightning bolt strikes the {} with a loud thunder! \
                 The damage is {} hit points.",
                objects[monster_id].name, LIGHTNING_DAMAGE
            ),
            LIGHT_BLUE,
        );
        objects[monster_id].take_damage(LIGHTNING_DAMAGE, game);
        UseResult::UsedUp
    } else {
        // no enemy found within maximum range
        game.messages
            .add("No enemy is close enough to strike.", RED);
        UseResult::Cancelled
    }
}

/// find closest enemy, up to a maximum range, and in the player's FOV
fn closest_monster(tcod: &Tcod, objects: &[Object], max_range: i32) -> Option<usize> {
    let mut closest_enemy = None;
    let mut closest_dist = (max_range + 1) as f32; // start with (slightly more than) maximum range

    for (id, object) in objects.iter().enumerate() {
        if (id != PLAYER)
            && object.fighter.is_some()
            && object.ai.is_some()
            && tcod.fov.is_in_fov(object.x, object.y)
        {
            // calculate distance between this object and the player
            let dist = objects[PLAYER].distance_to(object);
            if dist < closest_dist {
                // it's closer, so remember it
                closest_enemy = Some(id);
                closest_dist = dist;
            }
        }
    }
    closest_enemy
}

/// add to the player's inventory and remove from the map
fn pick_item_up(object_id: usize, game: &mut Game, objects: &mut Vec<Object>) {
    if game.inventory.len() >= 26 {
        game.messages.add(
            format!(
                "Your inventory is full, cannot pick up {}.",
                objects[object_id].name
            ),
            RED,
        );
    } else {
        let item = objects.swap_remove(object_id);
        game.messages
            .add(format!("You picked up a {}!", item.name), GREEN);
        game.inventory.push(item);
    }
}

/// return the position of a tile left-clicked in player's FOV (optionally in a
/// range), or (None,None) if right-clicked.
fn target_tile(
    tcod: &mut Tcod,
    game: &mut Game,
    objects: &[Object],
    max_range: Option<f32>,
) -> Option<(i32, i32)> {
    use tcod::input::KeyCode::Escape;
    loop {
        // render the screen. this erases the inventory and shows the names of
        // objects under the mouse.
        tcod.root.flush();
        let event = input::check_for_event(input::KEY_PRESS | input::MOUSE).map(|e| e.1);
        match event {
            Some(Event::Mouse(m)) => tcod.mouse = m,
            Some(Event::Key(k)) => tcod.key = k,
            None => tcod.key = Default::default(),
        }
        render_all(tcod, game, objects, false);

        let (x, y) = (tcod.mouse.cx as i32, tcod.mouse.cy as i32);

        // accept the target if the player clicked in FOV, and in case a range
        // is specified, if it's in that range
        let in_fov = (x < MAP_WIDTH) && (y < MAP_HEIGHT) && tcod.fov.is_in_fov(x, y);
        let in_range = max_range.map_or(true, |range| objects[PLAYER].distance(x, y) <= range);
        if tcod.mouse.lbutton_pressed && in_fov && in_range {
            return Some((x, y));
        }

        if tcod.mouse.rbutton_pressed || tcod.key.code == Escape {
            return None; // cancel if the player right-clicked or pressed Escape
        }
    }
}

// on player death
fn player_death(player: &mut Object, game: &mut Game) {
    // the game ended!
    game.messages.add("You are dead!", RED);

    // for added effect, transform the player into a corpse!
    player.char = '%';
    player.color = DARK_RED;
}

// on monster death
fn monster_death(monster: &mut Object, game: &mut Game) {
    // transform it into a nasty corpse! it doesn't block, can't be
    // attacked and doesn't move
    game.messages
        .add(format!("{} is dead!", monster.name), ORANGE);
    monster.char = '%';
    monster.color = DARK_RED;
    monster.blocks = false;
    monster.fighter = None;
    monster.ai = None;
    monster.name = format!("remains of {}", monster.name);
}

/// render a bar for the panel for health or mana or something
fn render_bar(
    panel: &mut Offscreen,
    x: i32,
    y: i32,
    total_width: i32,
    name: &str,
    value: i32,
    maximum: i32,
    bar_color: Color,
    back_color: Color,
) {
    // render a bar (HP, experience, etc). First calculate the width of the bar
    let bar_width = (value as f32 / maximum as f32 * total_width as f32) as i32;

    // render the background first
    panel.set_default_background(back_color);
    panel.rect(x, y, total_width, 1, false, BackgroundFlag::Screen);

    // now render the bar on top
    panel.set_default_background(bar_color);
    if bar_width > 0 {
        panel.rect(x, y, bar_width, 1, false, BackgroundFlag::Screen);
    }

    // finally, some centered text with the values
    panel.set_default_foreground(WHITE);
    panel.print_ex(
        x + total_width / 2,
        y,
        BackgroundFlag::None,
        TextAlignment::Center,
        &format!("{}: {}/{}", name, value, maximum),
    );
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PlayerAction {
    TookTurn,
    DidntTakeTurn,
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum DeathCallback {
    Player,
    Monster,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Item {
    Heal,
    Lightning,
    Confuse,
}

enum UseResult {
    UsedUp,
    Cancelled,
}

impl DeathCallback {
    fn callback(self, object: &mut Object, game: &mut Game) {
        use DeathCallback::*;
        let callback = match self {
            Player => player_death,
            Monster => monster_death,
        };
        callback(object, game);
    }
}

fn main() {
    tcod::system::set_fps(LIMIT_FPS);

    let root = Root::initializer()
        .font("resources/arial10x10.png", FontLayout::Tcod)
        .font_type(FontType::Greyscale)
        .size(SCREEN_WIDTH, SCREEN_HEIGHT)
        .title("Rust Rogue-Like")
        .init();

    let mut tcod = Tcod {
        root,
        console: Offscreen::new(MAP_WIDTH, MAP_HEIGHT),
        fov: FovMap::new(MAP_WIDTH, MAP_HEIGHT),
        panel: Offscreen::new(SCREEN_WIDTH, SCREEN_HEIGHT),
        key: Default::default(),
        mouse: Default::default(),
    };

    // create object representing the player
    let mut player = Object::new(0, 0, '@', "player", WHITE, true);
    player.alive = true;
    player.fighter = Some(Fighter {
        max_hp: 30,
        hp: 30,
        defense: 2,
        power: 5,
        on_death: DeathCallback::Player,
    });

    // the list of objects with just the player
    let mut objects = vec![player];

    let mut game = Game {
        // generate map (at this point it's not drawn to the screen)
        map: create_map(&mut objects),
        messages: Messages::new(),
        inventory: vec![],
    };

    // populate the FOV map, according to the generated map
    for y in 0..MAP_HEIGHT {
        for x in 0..MAP_WIDTH {
            tcod.fov.set(
                x,
                y,
                !game.map[x as usize][y as usize].block_sight,
                !game.map[x as usize][y as usize].blocked,
            );
        }
    }

    // force FOV "recompute" first time through the game loop
    let mut previous_player_position = (-1, -1);

    // a warm welcoming message!
    game.messages.add(
        "Welcome stranger! Prepare to perish in the Tombs of the Ancient Kings.",
        RED,
    );

    while !tcod.root.window_closed() {
        // clear the screen of the previous frame
        tcod.console.clear();

        // check for input
        match input::check_for_event(input::MOUSE | input::KEY_PRESS) {
            Some((_, Event::Mouse(m))) => tcod.mouse = m,
            Some((_, Event::Key(k))) => tcod.key = k,
            _ => tcod.key = Default::default(),
        }

        // render the screen
        let fov_recompute = previous_player_position != (objects[PLAYER].pos());
        render_all(&mut tcod, &mut game, &objects, fov_recompute);

        tcod.root.flush();

        // handle keys and exit game if needed
        previous_player_position = objects[PLAYER].pos();
        let player_action = handle_keys(&mut tcod, &mut game, &mut objects);
        if player_action == PlayerAction::Exit {
            break;
        }

        // let monsters take their turn
        if objects[PLAYER].alive && player_action != PlayerAction::DidntTakeTurn {
            for id in 0..objects.len() {
                if objects[id].ai.is_some() {
                    ai_take_turn(id, &tcod, &mut game, &mut objects);
                }
            }
        }
    }
}
