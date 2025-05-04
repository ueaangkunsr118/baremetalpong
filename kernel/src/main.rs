#![feature(sync_unsafe_cell)]
#![feature(abi_x86_interrupt)]
#![no_std]
#![no_main]

extern crate alloc;

mod screen;
mod allocator;
mod frame_allocator;
mod interrupts;
mod gdt;

use alloc::boxed::Box;
use alloc::format;
use core::fmt::Write;
use core::slice;
use bootloader_api::{entry_point, BootInfo, BootloaderConfig};
use bootloader_api::config::Mapping::Dynamic;
use bootloader_api::info::MemoryRegionKind;
use kernel::{HandlerTable, serial};
use pc_keyboard::{DecodedKey, KeyCode};
use x86_64::registers::control::Cr3;
use x86_64::VirtAddr;
use spin::Mutex;
use lazy_static::lazy_static;
use crate::frame_allocator::BootInfoFrameAllocator;
use crate::screen::{Writer, screenwriter};

const BOOTLOADER_CONFIG: BootloaderConfig = {
    let mut config = BootloaderConfig::new_default();
    config.mappings.physical_memory = Some(Dynamic);
    config.kernel_stack_size = 256 * 1024;
    config
};

entry_point!(kernel_main, config = &BOOTLOADER_CONFIG);

#[derive(PartialEq)]
enum GameMode {
    Menu,
    OnePlayer,
    TwoPlayer,
    GameOver,
}

struct PongGame {
    ball_x: isize,
    ball_y: isize,
    ball_dx: i8,
    ball_dy: i8,
    left_paddle: isize,
    right_paddle: isize,
    left_score: u8,
    right_score: u8,
    width: usize,
    height: usize,
    paddle_width: usize,
    paddle_height: usize,
    ball_size: usize,
    game_mode: GameMode,
    selected_menu_item: usize,
    max_ball_speed: i8,
    winner: Option<&'static str>,
}

impl PongGame {
    fn new(width: usize, height: usize) -> Self {
        PongGame {
            ball_x: (width / 2) as isize,
            ball_y: (height / 2) as isize,
            ball_dx: 70,
            ball_dy: 70,
            left_paddle: (height / 2) as isize,
            right_paddle: (height / 2) as isize,
            left_score: 0,
            right_score: 0,
            width,
            height,
            paddle_width: 15,
            paddle_height: 80,
            ball_size: 15,
            game_mode: GameMode::Menu,
            selected_menu_item: 0,
            max_ball_speed: 127,
            winner: None,
        }
    }

    fn update(&mut self) {
        if self.game_mode != GameMode::OnePlayer && self.game_mode != GameMode::TwoPlayer {
            return;
        }

        // Check for winner
        if self.left_score >= 3 {
            self.game_mode = GameMode::GameOver;
            self.winner = Some("PLAYER 1 WINS!");
            return;
        } else if self.right_score >= 3 {
            self.game_mode = GameMode::GameOver;
            self.winner = Some(
                if self.game_mode == GameMode::OnePlayer {
                    "CPU WINS!"
                } else {
                    "PLAYER 2 WINS!"
                }
            );
            return;
        }

        // Move ball
        self.ball_x += self.ball_dx as isize;
        self.ball_y += self.ball_dy as isize;

        // Wall collisions
        if self.ball_y <= 0 {
            self.ball_y = 0;
            self.ball_dy = self.ball_dy.abs();
        } else if self.ball_y >= (self.height - self.ball_size) as isize {
            self.ball_y = (self.height - self.ball_size) as isize;
            self.ball_dy = -self.ball_dy.abs();
        }

        // AI for single player
        if self.game_mode == GameMode::OnePlayer {
            let paddle_center = self.right_paddle + (self.paddle_height / 2) as isize;
            let ball_future_y = self.ball_y + (self.ball_dy as isize * 2);
            
            if paddle_center < ball_future_y - 5 {
                self.right_paddle = (self.right_paddle + 25).min((self.height - self.paddle_height) as isize);
            } else if paddle_center > ball_future_y + 5 {
                self.right_paddle = (self.right_paddle - 25).max(0);
            }
        }

        // Paddle collisions
        if self.ball_x <= self.paddle_width as isize {
            if self.ball_y + self.ball_size as isize >= self.left_paddle && 
               self.ball_y <= self.left_paddle + self.paddle_height as isize {
                self.ball_dx = (self.ball_dx.abs() + 5).min(self.max_ball_speed);
                self.ball_dy += (fast_rand() % 7) - 3;
            } else {
                self.right_score += 1;
                self.reset_ball();
            }
        } else if self.ball_x >= (self.width - self.paddle_width - self.ball_size) as isize {
            if self.ball_y + self.ball_size as isize >= self.right_paddle && 
               self.ball_y <= self.right_paddle + self.paddle_height as isize {
                self.ball_dx = -((self.ball_dx.abs() + 5).min(self.max_ball_speed));
                self.ball_dy += (fast_rand() % 7) - 3;
            } else {
                self.left_score += 1;
                self.reset_ball();
            }
        }

        // Speed limits
        self.ball_dx = self.ball_dx.clamp(-self.max_ball_speed, self.max_ball_speed);
        self.ball_dy = self.ball_dy.clamp(-self.max_ball_speed, self.max_ball_speed);
    }

    fn reset_ball(&mut self) {
        self.ball_x = (self.width / 2) as isize;
        self.ball_y = (self.height / 2) as isize;
        self.ball_dx = if fast_rand() % 2 == 0 { 100 } else { -100 };
        self.ball_dy = (fast_rand() % 15) - 7;
    }

    fn move_left_paddle(&mut self, up: bool) {
        if self.game_mode == GameMode::GameOver {
            return;
        }
        let move_amount = 25;
        self.left_paddle = if up {
            (self.left_paddle - move_amount).max(0)
        } else {
            (self.left_paddle + move_amount).min((self.height - self.paddle_height) as isize)
        };
    }

    fn move_right_paddle(&mut self, up: bool) {
        if self.game_mode == GameMode::GameOver {
            return;
        }
        let move_amount = 25;
        self.right_paddle = if up {
            (self.right_paddle - move_amount).max(0)
        } else {
            (self.right_paddle + move_amount).min((self.height - self.paddle_height) as isize)
        };
    }

    fn handle_menu_input(&mut self, key: DecodedKey) {
        match key {
            DecodedKey::Unicode('w') => {
                self.selected_menu_item = self.selected_menu_item.saturating_sub(1);
            }
            DecodedKey::Unicode('s') => {
                if self.selected_menu_item < 1 {
                    self.selected_menu_item += 1;
                }
            }
            DecodedKey::Unicode('\n') => {
                self.game_mode = match self.selected_menu_item {
                    0 => GameMode::OnePlayer,
                    1 => GameMode::TwoPlayer,
                    _ => GameMode::OnePlayer,
                };
                self.reset_ball();
                self.left_score = 0;
                self.right_score = 0;
                self.winner = None;
            }
            _ => {}
        }
    }

    fn draw(&self) {
        let mut writer = screenwriter();
        writer.clear_screen(0, 0, 0);

        match self.game_mode {
            GameMode::Menu => {
                writer.draw_string_centered(self.height / 2 - 60, "ULTRA PONG", 0xff, 0xff, 0xff);
                writer.draw_string_centered(
                    self.height / 2 - 20,
                    if self.selected_menu_item == 0 { "> 1 PLAYER <" } else { "  1 PLAYER  " },
                    0xff, 0xff, 0xff
                );
                writer.draw_string_centered(
                    self.height / 2,
                    if self.selected_menu_item == 1 { "> 2 PLAYERS <" } else { "  2 PLAYERS  " },
                    0xff, 0xff, 0xff
                );
                writer.draw_string_centered(self.height / 2 + 40, "CONTROLS:", 0x55, 0xff, 0x55);
                writer.draw_string_centered(self.height / 2 + 60, "PLAYER 1: W/S KEYS", 0xaa, 0xaa, 0xff);
                writer.draw_string_centered(self.height / 2 + 80, "PLAYER 2: I/K KEYS", 0xff, 0xaa, 0xaa);
                writer.draw_string_centered(self.height / 2 + 120, "FIRST TO 3 POINTS WINS!", 0xff, 0xff, 0x55);
                writer.draw_string_centered(self.height / 2 + 140, "MENU: W/S TO SELECT", 0xff, 0x55, 0x55);
                writer.draw_string_centered(self.height / 2 + 160, "ENTER TO START", 0x55, 0xff, 0x55);
            }
            GameMode::GameOver => {
                if let Some(winner) = self.winner {
                    writer.draw_string_centered(self.height / 2 - 40, winner, 0xff, 0xff, 0x55);
                }
                writer.draw_string_centered(self.height / 2, "GAME OVER", 0xff, 0x55, 0x55);
                writer.draw_string_centered(self.height / 2 + 40, "FINAL SCORE:", 0xff, 0xff, 0xff);
                let score_text = format!("{} - {}", self.left_score, self.right_score);
                writer.draw_string_centered(self.height / 2 + 70, &score_text, 0xff, 0xff, 0xff);
                writer.draw_string_centered(self.height / 2 + 120, "PRESS ENTER TO RETURN TO MENU", 0x55, 0xff, 0xff);
            }
            _ => {
                // Draw paddles
                for y in self.left_paddle as usize..(self.left_paddle + self.paddle_height as isize) as usize {
                    for x in 0..self.paddle_width {
                        writer.safe_draw_pixel(x, y, 0xff, 0xff, 0xff);
                    }
                }
                for y in self.right_paddle as usize..(self.right_paddle + self.paddle_height as isize) as usize {
                    for x in self.width - self.paddle_width..self.width {
                        writer.safe_draw_pixel(x, y, 0xff, 0xff, 0xff);
                    }
                }

                // Draw ball
                for y in self.ball_y as usize..(self.ball_y + self.ball_size as isize) as usize {
                    for x in self.ball_x as usize..(self.ball_x + self.ball_size as isize) as usize {
                        writer.safe_draw_pixel(x, y, 0xff, 0xff, 0xff);
                    }
                }

                // Draw center line
                for y in (0..self.height).step_by(20) {
                    writer.safe_draw_pixel(self.width / 2, y, 0x55, 0x55, 0x55);
                }

                // Draw scores
                let score_text = format!("{} - {}", self.left_score, self.right_score);
                writer.draw_string_centered(20, &score_text, 0xff, 0xff, 0xff);
                
                // Draw speed indicator
                let speed = self.ball_dx.abs().max(self.ball_dy.abs());
                let speed_text = format!("SPEED: {}/{}", speed, self.max_ball_speed);
                writer.draw_string(10, 10, &speed_text, 0x55, 0xff, 0x55);
            }
        }
    }
}

fn fast_rand() -> i8 {
    static mut SEED: u32 = 42;
    unsafe {
        SEED = SEED.wrapping_mul(1664525).wrapping_add(1013904223);
        (SEED >> 16) as i8
    }
}

lazy_static! {
    static ref GAME_STATE: Mutex<PongGame> = Mutex::new(PongGame::new(0, 0));
}

fn handle_keyboard_input(key: DecodedKey) {
    let mut game = GAME_STATE.lock();
    
    match game.game_mode {
        GameMode::Menu => game.handle_menu_input(key),
        GameMode::OnePlayer => match key {
            DecodedKey::Unicode('w') => game.move_left_paddle(true),
            DecodedKey::Unicode('s') => game.move_left_paddle(false),
            DecodedKey::Unicode('\n') if game.game_mode == GameMode::GameOver => {
                game.game_mode = GameMode::Menu;
            }
            _ => (),
        },
        GameMode::TwoPlayer => match key {
            DecodedKey::Unicode('w') => game.move_left_paddle(true),
            DecodedKey::Unicode('s') => game.move_left_paddle(false),
            DecodedKey::Unicode('i') => game.move_right_paddle(true),
            DecodedKey::Unicode('k') => game.move_right_paddle(false),
            DecodedKey::Unicode('\n') if game.game_mode == GameMode::GameOver => {
                game.game_mode = GameMode::Menu;
            }
            _ => (),
        },
        GameMode::GameOver => {
            if let DecodedKey::Unicode('\n') = key {
                game.game_mode = GameMode::Menu;
            }
        }
    }
}

fn update_game() {
    let mut game = GAME_STATE.lock();
    game.update();
    game.draw();
}

fn kernel_main(boot_info: &'static mut BootInfo) -> ! {
    writeln!(serial(), "Entered kernel with boot info: {boot_info:?}").unwrap();

    let frame_info = boot_info.framebuffer.as_ref().unwrap().info();
    let framebuffer = boot_info.framebuffer.as_mut().unwrap();
    screen::init(framebuffer);

    *GAME_STATE.lock() = PongGame::new(frame_info.width as usize, frame_info.height as usize);

    for r in boot_info.memory_regions.iter() {
        writeln!(serial(), "{:?} {:?} {:?} {}", r, r.start as *mut u8, r.end as *mut usize, r.end-r.start).unwrap();
    }

    let usable_region = boot_info.memory_regions.iter()
        .filter(|x|x.kind == MemoryRegionKind::Usable)
        .last()
        .unwrap();
    
    let physical_offset = boot_info.physical_memory_offset.take().expect("Failed to find physical memory offset");
    allocator::init_heap((physical_offset + usable_region.start) as usize);

    let rsdp = boot_info.rsdp_addr.take();
    let mut mapper = frame_allocator::init(VirtAddr::new(physical_offset));
    let mut frame_allocator = BootInfoFrameAllocator::new(&boot_info.memory_regions);
    
    gdt::init();
    
    let lapic_ptr = interrupts::init_apic(
        rsdp.expect("Failed to get RSDP address") as usize,
        physical_offset,
        &mut mapper,
        &mut frame_allocator
    );

    HandlerTable::new()
        .keyboard(handle_keyboard_input)
        .timer(update_game)
        .startup(|| {
            writeln!(Writer, "Pong Game Initialized!").unwrap();
        })
        .start(lapic_ptr)
}