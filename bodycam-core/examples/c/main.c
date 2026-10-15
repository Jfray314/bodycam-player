/* Loads a weapon JSON, runs 2 s at 60 fps (fire held, ADS from t=1 s), prints viewmodel offsets and spread.
 * usage: main <weapon_stats.json> <weapon.json> */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bodycam_core.h"

static char* slurp(const char* p) {
    FILE* f = fopen(p, "rb"); if (!f) return NULL;
    fseek(f, 0, SEEK_END); long n = ftell(f); rewind(f);
    char* b = malloc(n + 1); b[fread(b, 1, n, f)] = 0; fclose(f); return b;
}

int main(int argc, char** argv) {
    if (argc < 3) return 2;
    char *sj = slurp(argv[1]), *wj = slurp(argv[2]);
    if (!sj || !wj) { puts("cannot read json"); return 1; }
    BcStats* st = bc_stats_load(sj); BcWeapon* w = bc_weapon_load(st, wj);
    if (!w) { puts("weapon load failed"); return 1; }
    BcWeaponInfo wi; bc_weapon_info(w, &wi);
    printf("damage %.1f rate %.3fs pellets %u ammo %u auto %u\n", wi.damage, wi.fire_rate, wi.pellets, wi.ammo_capacity, wi.full_auto);
    printf("spread hip %.2f/%.2f deg, aim %.2f/%.2f deg\n", wi.spread_hip[0], wi.spread_hip[1], wi.spread_aim[0], wi.spread_aim[1]);
    BcSway* sw = bc_sway_create(); BcRecoil* rc = bc_recoil_create(1); BcGun* g = bc_gun_create(w, wi.ammo_capacity, 1);
    int shots = 0; float aim[3] = {0, 0, -1}, dirs[3 * 64] = {0};
    for (int i = 0; i < 120; i++) {
        float t = i / 60.f; uint32_t aiming = t >= 1.f;
        BcFireOut fo; bc_gun_step(g, 1 / 60.f, t, BC_TRIG_HELD | (i == 0 ? BC_TRIG_PRESSED : 0) | (aiming ? BC_TRIG_AIMING : 0), &fo);
        if (fo.fired) { shots++; bc_recoil_play(rc, w, aiming); bc_gun_dirs(g, aim, aiming, dirs, 64); }
        BcSwayIn si = { 1 / 60.f, i % 20 < 10 ? 2.f : -2.f, 0, 3.f, 0, 0, aiming, 0, {0} }; /* look +-2 deg, walking 3 m/s */ BcSwayOut so; bc_sway_step(sw, w, &si, &so);
        BcRecoilOut ro; bc_recoil_tick(rc, 1 / 60.f, &ro);
        if (i % 30 == 29) printf("t=%.2f ads=%.2f vm=(%.4f %.4f %.4f) rot=(%.2f %.2f %.2f) recoil=(%.4f %.4f %.4f) shots=%d last_dir=(%.4f %.4f %.4f)\n", t, so.ads_alpha,
            so.translation[0], so.translation[1], so.translation[2], so.rot_deg[0], so.rot_deg[1], so.rot_deg[2], ro.translation[0], ro.translation[1], ro.translation[2], shots, dirs[0], dirs[1], dirs[2]);
    }
    printf("ammo left %u\n", bc_gun_ammo(g));
    bc_gun_destroy(g); bc_recoil_destroy(rc); bc_sway_destroy(sw); bc_weapon_free(w); bc_stats_free(st); return 0;
}
