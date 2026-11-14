// SPDX-License-Identifier: GPL-3.0-or-later
// Play mode pad orientation (src/gfx/play/pad_orient.cpp): the gyro turns the pad's axes, the accelerometer pulls the tilt back toward gravity.
#include "test_framework.h"

#include <algorithm>
#include <cmath>
#include <string>

#include "play/pad_orient.h"

NXTEST_GROUP(play_orient)

namespace {

namespace play = nxrt::play;

constexpr double kPi = 3.14159265358979323846;

bool close_to(double a, double b, double eps) { return std::fabs(a - b) <= eps; }

// One row of the direction matrix (the pad's axis `row` in the world frame) against the expected world vector, to 1e-3.
void check_row(const float d[9], int row, double x, double y, double z, const char* what) {
    const double want[3] = {x, y, z};
    for (int w = 0; w < 3; ++w)
        CHECK_MSG(close_to(d[3 * row + w], want[w], 1e-3),
                  std::string(what) + " component " + std::to_string(w) + " is " + std::to_string(d[3 * row + w]));
}

// Runs `steps` calls of step() with the same gyro and accel.
void run_steps(play::PadOrient& o, const float gyro[3], const float accel[3], int steps, double dt, double tilt_gain) {
    for (int i = 0; i < steps; ++i) o.step(gyro, accel, dt, tilt_gain);
}

// Sets the pad to a tilt of deg degrees about the world x axis. The rows are the columns of M = Rx(deg), the layout pad_orient.cpp uses
// (direction[3a + w] = M[w][a]): x row (1, 0, 0), y row (0, c, s), z row (0, -s, c).
void set_tilt_x(play::PadOrient& o, double deg) {
    const double r = deg * kPi / 180.0;
    const float c = static_cast<float>(std::cos(r));
    const float s = static_cast<float>(std::sin(r));
    const float rows[9] = {1.f, 0.f, 0.f, 0.f, c, s, 0.f, -s, c};
    for (int i = 0; i < 9; ++i) o.direction[i] = rows[i];
}

// Angle in radians between the pad's z row and world up (0, 0, 1).
double z_tilt(const float d[9]) {
    const double zx = d[6], zy = d[7], zz = d[8];
    return std::atan2(std::sqrt(zx * zx + zy * zy), zz);
}

// Largest change of any direction element after one zero-gyro step from a 20 degree tilt about x, with the given accel reading.
double one_step_change(const float accel[3]) {
    play::PadOrient o;
    o.reset();
    set_tilt_x(o, 20.0);
    float before[9];
    for (int i = 0; i < 9; ++i) before[i] = o.direction[i];
    const float gyro[3] = {0.f, 0.f, 0.f};
    o.step(gyro, accel, 0.005, 0.5);
    double worst = 0.0;
    for (int i = 0; i < 9; ++i) worst = std::max(worst, std::fabs(static_cast<double>(o.direction[i]) - before[i]));
    return worst;
}

}  // namespace

NXTEST(identity_after_reset) {
    const play::PadOrient fresh{};
    const float id[9] = {1, 0, 0, 0, 1, 0, 0, 0, 1};
    for (int i = 0; i < 9; ++i) CHECK_MSG(close_to(fresh.direction[i], id[i], 1e-12), "fresh element " + std::to_string(i));

    // Move it off identity, then reset.
    play::PadOrient o;
    const float gyro[3] = {0.3f, -0.2f, 0.1f};
    const float accel[3] = {0.f, 0.f, -1.f};
    run_steps(o, gyro, accel, 50, 0.005, 0.5);
    o.reset();
    for (int i = 0; i < 9; ++i) CHECK_MSG(close_to(o.direction[i], id[i], 1e-12), "reset element " + std::to_string(i));
    for (int i = 0; i < 3; ++i) CHECK_MSG(close_to(o.angle[i], 0.0, 1e-12), "reset angle " + std::to_string(i));
}

NXTEST(gyro_quarter_turn_about_z) {
    // Rotation by +90 degrees about z takes x to +y and y to -x.
    play::PadOrient o;
    o.reset();
    const float gyro[3] = {0.f, 0.f, 0.25f};
    const float accel[3] = {0.f, 0.f, -1.f};
    run_steps(o, gyro, accel, 200, 0.005, 0.0);   // 1 s at 0.25 rev/s
    check_row(o.direction, 0, 0, 1, 0, "x row");
    check_row(o.direction, 1, -1, 0, 0, "y row");
    check_row(o.direction, 2, 0, 0, 1, "z row");
    CHECK_MSG(close_to(o.angle[2], 0.25, 1e-4), "angle z is " + std::to_string(o.angle[2]));
    CHECK_MSG(close_to(o.angle[0], 0.0, 1e-4) && close_to(o.angle[1], 0.0, 1e-4), "angle x or y moved");
}

NXTEST(gyro_quarter_turn_about_x) {
    // Rotation by +90 degrees about x takes y to +z and z to -y.
    play::PadOrient o;
    o.reset();
    const float gyro[3] = {0.25f, 0.f, 0.f};
    const float accel[3] = {0.f, 0.f, -1.f};
    run_steps(o, gyro, accel, 200, 0.005, 0.0);
    check_row(o.direction, 0, 1, 0, 0, "x row");
    check_row(o.direction, 1, 0, 0, 1, "y row");
    check_row(o.direction, 2, 0, -1, 0, "z row");
    CHECK_MSG(close_to(o.angle[0], 0.25, 1e-4), "angle x is " + std::to_string(o.angle[0]));
    CHECK_MSG(close_to(o.angle[1], 0.0, 1e-4) && close_to(o.angle[2], 0.0, 1e-4), "angle y or z moved");
}

NXTEST(stays_orthonormal_over_many_steps) {
    play::PadOrient o;
    o.reset();
    for (int i = 0; i < 20000; ++i) {
        const double t = i;
        const float gyro[3] = {static_cast<float>(0.8 * std::sin(0.37 * t)), static_cast<float>(0.6 * std::cos(0.91 * t)),
                               static_cast<float>(0.7 * std::sin(1.3 * t + 0.5))};
        const float accel[3] = {static_cast<float>(0.1 * std::sin(0.5 * t)), static_cast<float>(0.1 * std::cos(0.3 * t)), -1.f};
        o.step(gyro, accel, 0.005, 0.5);
    }
    for (int a = 0; a < 3; ++a) {
        const float* r = &o.direction[3 * a];
        const double len = std::sqrt(static_cast<double>(r[0]) * r[0] + static_cast<double>(r[1]) * r[1] + static_cast<double>(r[2]) * r[2]);
        CHECK_MSG(close_to(len, 1.0, 1e-4), "row " + std::to_string(a) + " length " + std::to_string(len));
    }
    for (int a = 0; a < 3; ++a) {
        for (int b = a + 1; b < 3; ++b) {
            const float* p = &o.direction[3 * a];
            const float* q = &o.direction[3 * b];
            const double dot = static_cast<double>(p[0]) * q[0] + static_cast<double>(p[1]) * q[1] + static_cast<double>(p[2]) * q[2];
            CHECK_MSG(std::fabs(dot) <= 1e-4, "rows " + std::to_string(a) + " and " + std::to_string(b) + " dot " + std::to_string(dot));
        }
    }
}

NXTEST(tilt_correction_levels_out) {
    // Start 20 degrees tilted about x, but the pad lies flat (accel 0, 0, -1) with no gyro: the tilt must pull back to level.
    // Rate: the tilt angle shrinks as dθ/dt = -tilt_gain * sin(θ), so after 10 s at 0.5 it is 20 deg * e^-5, about 0.13 deg (0.0024 rad).
    play::PadOrient o;
    o.reset();
    set_tilt_x(o, 20.0);
    const float gyro[3] = {0.f, 0.f, 0.f};
    const float accel[3] = {0.f, 0.f, -1.f};
    double prev = z_tilt(o.direction);
    CHECK_MSG(close_to(prev, 20.0 * kPi / 180.0, 1e-5), "start tilt is " + std::to_string(prev));
    for (int i = 0; i < 2000; ++i) {   // 10 s at 5 ms
        o.step(gyro, accel, 0.005, 0.5);
        const double now = z_tilt(o.direction);
        CHECK_MSG(now <= prev + 1e-6, "tilt grew at step " + std::to_string(i) + ": " + std::to_string(now));
        prev = now;
    }
    const float* z = &o.direction[6];
    const double dist = std::sqrt(static_cast<double>(z[0]) * z[0] + static_cast<double>(z[1]) * z[1] + (static_cast<double>(z[2]) - 1.0) * (z[2] - 1.0));
    CHECK_MSG(dist <= 0.02, "z row after 10 s is " + std::to_string(z[0]) + " " + std::to_string(z[1]) + " " + std::to_string(z[2]));
}

NXTEST(tilted_pad_reading_its_own_gravity_stays_put) {
    // A pad tilted 20 degrees about x reads gravity as accel = M^T (0, 0, -1) = (0, -sin, -cos), which is already consistent with its
    // tilt, so the correction must do nothing. This pins the accel sign: the opposite sign would move it.
    play::PadOrient o;
    o.reset();
    set_tilt_x(o, 20.0);
    const float s = static_cast<float>(std::sin(20.0 * kPi / 180.0));
    const float c = static_cast<float>(std::cos(20.0 * kPi / 180.0));
    const float accel[3] = {0.f, -s, -c};
    const float gyro[3] = {0.f, 0.f, 0.f};
    float before[9];
    for (int i = 0; i < 9; ++i) before[i] = o.direction[i];
    run_steps(o, gyro, accel, 1000, 0.005, 0.5);
    for (int i = 0; i < 9; ++i) CHECK_MSG(close_to(o.direction[i], before[i], 1e-5), "element " + std::to_string(i) + " moved");
}

NXTEST(bad_dt_changes_nothing) {
    play::PadOrient o;
    o.reset();
    const float gyro[3] = {0.1f, 0.2f, 0.3f};
    const float accel[3] = {0.f, 0.f, -1.f};
    o.step(gyro, accel, 0.01);   // off identity, so a wrongly accepted step would show
    float d0[9];
    for (int i = 0; i < 9; ++i) d0[i] = o.direction[i];
    const double a0[3] = {o.angle[0], o.angle[1], o.angle[2]};
    o.step(gyro, accel, 0.0);
    o.step(gyro, accel, -0.01);
    o.step(gyro, accel, 0.6);
    o.step(gyro, accel, 5.0);
    for (int i = 0; i < 9; ++i) CHECK_MSG(close_to(o.direction[i], d0[i], 1e-9), "direction " + std::to_string(i) + " changed");
    for (int i = 0; i < 3; ++i) CHECK_MSG(close_to(o.angle[i], a0[i], 1e-9), "angle " + std::to_string(i) + " changed");
}

NXTEST(tilt_correction_needs_about_one_g) {
    const float flat[3] = {0.f, 0.f, -1.f};
    const float none[3] = {0.f, 0.f, 0.f};
    const float heavy[3] = {0.f, 0.f, -2.f};
    // control: inside the 0.85 to 1.15 G band the same tilted start is corrected
    CHECK_MSG(one_step_change(flat) > 1e-5, "control: one G did not correct the tilt");
    // outside the band the correction is off, so a step with no gyro leaves the matrix as it was
    CHECK_MSG(one_step_change(none) <= 1e-6, "zero accel still corrected, change " + std::to_string(one_step_change(none)));
    CHECK_MSG(one_step_change(heavy) <= 1e-6, "2 G still corrected, change " + std::to_string(one_step_change(heavy)));
}
