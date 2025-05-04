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
enum GameState {
    TitleScreen,
    SinglePlayer,
    MultiPlayer,
    EndScreen,
}

struct PongGame {
    ball_x: isize,
    ball_y: isize,
    ball_speed_x: i8,
    ball_speed_y: i8,
    player1_position: isize,
    player2_position: isize,
    player1_score: u8,
    player2_score: u8,
    arena_width: usize,
    arena_height: usize,
    controller_width: usize,
    controller_height: usize,
    ball_size: usize,
    game_state: GameState,
    menu_selection: usize,
    speed_cap: i8,
    champion: Option<&'static str>,
}

impl PongGame {
    fn new(width: usize, height: usize) -> Self {
        PongGame {
            ball_x: (width / 2) as isize,
            ball_y: (height / 2) as isize,
            ball_speed_x: 70,
            ball_speed_y: 70,
            player1_position: (height / 2) as isize,
            player2_position: (height / 2) as isize,
            player1_score: 0,
            player2_score: 0,
            arena_width: width,
            arena_height: height,
            controller_width: 15,
            controller_height: 80,
            ball_size: 15,
            game_state: GameState::TitleScreen,
            menu_selection: 0,
            speed_cap: 127,
            champion: None,
        }
    }

    fn update(&mut self) {
        if self.game_state != GameState::SinglePlayer && self.game_state != GameState::MultiPlayer {
            return;
        }

        // Check for winner
        if self.player1_score >= 3 {
            self.game_state = GameState::EndScreen;
            self.champion = Some("PLAYER 1 VICTORIOUS!");
            return;
        } else if self.player2_score >= 3 {
            self.game_state = GameState::EndScreen;
            self.champion = Some(
                if self.game_state == GameState::SinglePlayer {
                    "AI VICTORIOUS!"
                } else {
                    "PLAYER 2 VICTORIOUS!"
                }
            );
            return;
        }

        // Move ball
        self.ball_x += self.ball_speed_x as isize;
        self.ball_y += self.ball_speed_y as isize;

        // Wall collisions
        if self.ball_y <= 0 {
            self.ball_y = 0;
            self.ball_speed_y = self.ball_speed_y.abs();
        } else if self.ball_y >= (self.arena_height - self.ball_size) as isize {
            self.ball_y = (self.arena_height - self.ball_size) as isize;
            self.ball_speed_y = -self.ball_speed_y.abs();
        }

        // AI for single player
        if self.game_state == GameState::SinglePlayer {
            let controller_center = self.player2_position + (self.controller_height / 2) as isize;
            let ball_future_y = self.ball_y + (self.ball_speed_y as isize * 2);
            
            if controller_center < ball_future_y - 5 {
                self.player2_position = (self.player2_position + 25).min((self.arena_height - self.controller_height) as isize);
            } else if controller_center > ball_future_y + 5 {
                self.player2_position = (self.player2_position - 25).max(0);
            }
        }

        // Paddle collisions
        if self.ball_x <= self.controller_width as isize {
            if self.ball_y + self.ball_size as isize >= self.player1_position && 
               self.ball_y <= self.player1_position + self.controller_height as isize {
                self.ball_speed_x = (self.ball_speed_x.abs() + 5).min(self.speed_cap);
                self.ball_speed_y += (chaos_number() % 7) - 3;
            } else {
                self.player2_score += 1;
                self.reset_ball();
            }
        } else if self.ball_x >= (self.arena_width - self.controller_width - self.ball_size) as isize {
            if self.ball_y + self.ball_size as isize >= self.player2_position && 
               self.ball_y <= self.player2_position + self.controller_height as isize {
                self.ball_speed_x = -((self.ball_speed_x.abs() + 5).min(self.speed_cap));
                self.ball_speed_y += (chaos_number() % 7) - 3;
            } else {
                self.player1_score += 1;
                self.reset_ball();
            }
        }

        // Speed limits
        self.ball_speed_x = self.ball_speed_x.clamp(-self.speed_cap, self.speed_cap);
        self.ball_speed_y = self.ball_speed_y.clamp(-self.speed_cap, self.speed_cap);
    }

    fn reset_ball(&mut self) {
        self.ball_x = (self.arena_width / 2) as isize;
        self.ball_y = (self.arena_height / 2) as isize;
        self.ball_speed_x = if chaos_number() % 2 == 0 { 100 } else { -100 };
        self.ball_speed_y = (chaos_number() % 15) - 7;
    }

    fn move_player1(&mut self, up: bool) {
        if self.game_state == GameState::EndScreen {
            return;
        }
        let move_amount = 25;
        self.player1_position = if up {
            (self.player1_position - move_amount).max(0)
        } else {
            (self.player1_position + move_amount).min((self.arena_height - self.controller_height) as isize)
        };
    }

    fn move_player2(&mut self, up: bool) {
        if self.game_state == GameState::EndScreen {
            return;
        }
        let move_amount = 25;
        self.player2_position = if up {
            (self.player2_position - move_amount).max(0)
        } else {
            (self.player2_position + move_amount).min((self.arena_height - self.controller_height) as isize)
        };
    }

    fn handle_menu_input(&mut self, key: DecodedKey) {
        match key {
            DecodedKey::Unicode('w') => {
                self.menu_selection = self.menu_selection.saturating_sub(1);
            }
            DecodedKey::Unicode('s') => {
                if self.menu_selection < 1 {
                    self.menu_selection += 1;
                }
            }
            DecodedKey::Unicode('\n') => {
                self.game_state = match self.menu_selection {
                    0 => GameState::SinglePlayer,
                    1 => GameState::MultiPlayer,
                    _ => GameState::SinglePlayer,
                };
                self.reset_ball();
                self.player1_score = 0;
                self.player2_score = 0;
                self.champion = None;
            }
            _ => {}
        }
    }

    fn draw(&self) {
        let mut writer = screenwriter();
        writer.clear_screen(0, 0, 20); // Dark blue background

        match self.game_state {
            GameState::TitleScreen => {
                writer.draw_string_centered(self.arena_height / 2 - 80, "NEON PONG ARENA", 0x20, 0xff, 0xd0);
                writer.draw_string_centered(
                    self.arena_height / 2 - 20,
                    if self.menu_selection == 0 { "> SINGLE PLAYER <" } else { "  SINGLE PLAYER  " },
                    0x50, 0xf0, 0xff
                );
                writer.draw_string_centered(
                    self.arena_height / 2,
                    if self.menu_selection == 1 { "> VERSUS MODE <" } else { "  VERSUS MODE  " },
                    0x50, 0xf0, 0xff
                );
                writer.draw_string_centered(self.arena_height / 2 + 40, "CONTROL SCHEME:", 0x55, 0xff, 0x99);
                writer.draw_string_centered(self.arena_height / 2 + 60, "PLAYER 1: W/S KEYS", 0x99, 0xcc, 0xff);
                writer.draw_string_centered(self.arena_height / 2 + 80, "PLAYER 2: I/K KEYS", 0xff, 0x99, 0xcc);
                writer.draw_string_centered(self.arena_height / 2 + 120, "BEST OF 3 WINS THE MATCH!", 0xff, 0xff, 0x75);
                writer.draw_string_centered(self.arena_height / 2 + 140, "NAVIGATE: W/S TO SELECT", 0xff, 0x75, 0x75);
                writer.draw_string_centered(self.arena_height / 2 + 160, "PRESS ENTER TO BEGIN", 0x75, 0xff, 0x75);
            }
            GameState::EndScreen => {
                if let Some(winner) = self.champion {
                    writer.draw_string_centered(self.arena_height / 2 - 40, winner, 0xff, 0xff, 0x75);
                }
                writer.draw_string_centered(self.arena_height / 2, "MATCH COMPLETE", 0xff, 0x75, 0x75);
                writer.draw_string_centered(self.arena_height / 2 + 40, "FINAL SCORE:", 0xff, 0xff, 0xff);
                let score_text = format!("{} - {}", self.player1_score, self.player2_score);
                writer.draw_string_centered(self.arena_height / 2 + 70, &score_text, 0xff, 0xff, 0xff);
                writer.draw_string_centered(self.arena_height / 2 + 120, "PRESS ENTER TO RETURN TO MENU", 0x75, 0xff, 0xff);
            }
            _ => {
                // Draw paddles
                for y in self.player1_position as usize..(self.player1_position + self.controller_height as isize) as usize {
                    for x in 0..self.controller_width {
                        writer.safe_draw_pixel(x, y, 0x50, 0xf0, 0xff);
                    }
                }
                for y in self.player2_position as usize..(self.player2_position + self.controller_height as isize) as usize {
                    for x in self.arena_width - self.controller_width..self.arena_width {
                        writer.safe_draw_pixel(x, y, 0xff, 0x50, 0xf0);
                    }
                }

                // Draw ball
                for y in self.ball_y as usize..(self.ball_y + self.ball_size as isize) as usize {
                    for x in self.ball_x as usize..(self.ball_x + self.ball_size as isize) as usize {
                        writer.safe_draw_pixel(x, y, 0xff, 0xff, 0x50);
                    }
                }

                // Draw center line
                for y in (0..self.arena_height).step_by(20) {
                    writer.safe_draw_pixel(self.arena_width / 2, y, 0x80, 0x80, 0x80);
                }

                // Draw scores
                let score_text = format!("{} - {}", self.player1_score, self.player2_score);
                writer.draw_string_centered(20, &score_text, 0xff, 0xff, 0xff);
                
                // Draw speed indicator
                let speed = self.ball_speed_x.abs().max(self.ball_speed_y.abs());
                let speed_text = format!("SPEED: {}/{}", speed, self.speed_cap);
                writer.draw_string(10, 10, &speed_text, 0x75, 0xff, 0x75);
            }
        }
    }
}

fn chaos_number() -> i8 {
    static mut ENTROPY: u32 = 42;
    unsafe {
        ENTROPY = ENTROPY.wrapping_mul(1664525).wrapping_add(1013904223);
        (ENTROPY >> 16) as i8
    }
}

lazy_static! {
    static ref GAME_STATE: Mutex<PongGame> = Mutex::new(PongGame::new(0, 0));
}

fn handle_keyboard_input(key: DecodedKey) {
    let mut game = GAME_STATE.lock();
    
    match game.game_state {
        GameState::TitleScreen => game.handle_menu_input(key),
        GameState::SinglePlayer => match key {
            DecodedKey::Unicode('w') => game.move_player1(true),
            DecodedKey::Unicode('s') => game.move_player1(false),
            DecodedKey::Unicode('\n') if game.game_state == GameState::EndScreen => {
                game.game_state = GameState::TitleScreen;
            }
            _ => (),
        },
        GameState::MultiPlayer => match key {
            DecodedKey::Unicode('w') => game.move_player1(true),
            DecodedKey::Unicode('s') => game.move_player1(false),
            DecodedKey::Unicode('i') => game.move_player2(true),
            DecodedKey::Unicode('k') => game.move_player2(false),
            DecodedKey::Unicode('\n') if game.game_state == GameState::EndScreen => {
                game.game_state = GameState::TitleScreen;
            }
            _ => (),
        },
        GameState::EndScreen => {
            if let DecodedKey::Unicode('\n') = key {
                game.game_state = GameState::TitleScreen;
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
            writeln!(Writer, "Neon Pong Arena Initialized!").unwrap();
        })
        .start(lapic_ptr)
}