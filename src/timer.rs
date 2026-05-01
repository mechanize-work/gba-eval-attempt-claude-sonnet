use crate::Gba;

const PRESCALER: [u32; 4] = [1, 64, 256, 1024];

impl Gba {
    pub(crate) fn timer_write_ctrl(&mut self, t: usize, val: u16) {
        let was_enabled = self.timers[t].enabled;
        let now_enabled = (val >> 7) & 1 != 0;

        self.timers[t].ctrl = val;
        self.timers[t].enabled = now_enabled;
        self.timers[t].cascade = (val >> 2) & 1 != 0;
        self.timers[t].irq = (val >> 6) & 1 != 0;
        self.timers[t].prescaler = PRESCALER[(val & 3) as usize];

        if now_enabled && !was_enabled {
            // Reload counter when timer is first enabled
            self.timers[t].counter = self.timers[t].reload;
            self.timers[t].cycles = 0;
        }
    }

    pub(crate) fn tick_timers(&mut self, cycles: u32) {
        for i in 0..4 {
            if !self.timers[i].enabled { continue; }
            if self.timers[i].cascade && i > 0 { continue; }  // cascade timers tick on overflow

            self.timers[i].cycles += cycles;
            while self.timers[i].cycles >= self.timers[i].prescaler {
                self.timers[i].cycles -= self.timers[i].prescaler;
                self.timer_tick(i);
            }
        }
    }

    fn timer_tick(&mut self, t: usize) {
        let (new_counter, overflow) = self.timers[t].counter.overflowing_add(1);
        if overflow {
            self.timers[t].counter = self.timers[t].reload;
            self.on_timer_overflow(t);
        } else {
            self.timers[t].counter = new_counter;
        }
    }

    fn on_timer_overflow(&mut self, t: usize) {
        // Generate IRQ if enabled
        if self.timers[t].irq {
            self.if_ |= 1 << (3 + t);
        }

        // APU uses timer 0 and 1 for DMA audio
        if t <= 1 {
            self.apu_timer_overflow(t);
        }

        // Cascade to next timer
        if t < 3 && self.timers[t + 1].enabled && self.timers[t + 1].cascade {
            self.timer_tick(t + 1);
        }
    }
}
