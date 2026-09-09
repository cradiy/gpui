#[derive(Clone, Copy)]
pub(super) struct Rotation([f64; 4]);

impl Rotation {
    pub const IDENTITY: Self = Self([0., 0., 0., 1.]);

    fn normalized(mut q: [f64; 4]) -> Self {
        let length = q.iter().map(|value| value * value).sum::<f64>().sqrt();
        let scale = if q[3] < 0. { -1. / length } else { 1. / length };
        q.iter_mut().for_each(|value| *value *= scale);
        Self(q)
    }

    pub fn from_matrix(m: [[f64; 3]; 3]) -> Self {
        let trace = m[0][0] + m[1][1] + m[2][2];
        let q = if trace > 0. {
            let s = 2. * (1. + trace).sqrt();
            [
                (m[1][2] - m[2][1]) / s,
                (m[2][0] - m[0][2]) / s,
                (m[0][1] - m[1][0]) / s,
                s * 0.25,
            ]
        } else {
            let i = (0..3).max_by(|&a, &b| m[a][a].total_cmp(&m[b][b])).unwrap();
            let j = (i + 1) % 3;
            let k = (i + 2) % 3;
            let s = 2. * (1. + m[i][i] - m[j][j] - m[k][k]).max(0.).sqrt();
            let mut q = [0.; 4];
            q[i] = s * 0.25;
            q[j] = (m[i][j] + m[j][i]) / s;
            q[k] = (m[i][k] + m[k][i]) / s;
            q[3] = (m[j][k] - m[k][j]) / s;
            q
        };
        Self::normalized(q)
    }

    pub fn between(from: [f64; 3], to: [f64; 3]) -> Self {
        let perpendicular = cross(from, to);
        let sine = dot(perpendicular, perpendicular).sqrt();
        let cosine = dot(from, to).clamp(-1., 1.);
        if sine <= 1e-12 {
            if cosine >= 0. {
                return Self::IDENTITY;
            }
            let i = (0..3)
                .min_by(|&a, &b| from[a].abs().total_cmp(&from[b].abs()))
                .unwrap();
            let mut reference = [0.; 3];
            reference[i] = 1.;
            let axis = unit(cross(from, reference)).unwrap();
            return Self([axis[0], axis[1], axis[2], 0.]);
        }
        let angle = sine.atan2(cosine) * 0.5;
        let axis = perpendicular.map(|value| value * angle.sin() / sine);
        Self::normalized([axis[0], axis[1], axis[2], angle.cos()])
    }

    pub fn angle(self) -> f64 {
        let [x, y, z, w] = self.0;
        2. * dot([x, y, z], [x, y, z]).sqrt().atan2(w)
    }

    pub fn scaled(self, weight: f64) -> Self {
        let [x, y, z, _] = self.0;
        let sine = dot([x, y, z], [x, y, z]).sqrt();
        if sine == 0. || weight == 0. {
            return Self::IDENTITY;
        }
        let half = self.angle() * weight * 0.5;
        let scale = half.sin() / sine;
        Self([x * scale, y * scale, z * scale, half.cos()])
    }

    pub fn inverse(self) -> Self {
        let [x, y, z, w] = self.0;
        Self([-x, -y, -z, w])
    }

    pub fn compose(self, other: Self) -> Self {
        let [x, y, z, w] = self.0;
        let [a, b, c, d] = other.0;
        Self::normalized([
            w * a + x * d + y * c - z * b,
            w * b - x * c + y * d + z * a,
            w * c + x * b - y * a + z * d,
            w * d - x * a - y * b - z * c,
        ])
    }

    pub fn matrix(self) -> [[f64; 3]; 3] {
        let [x, y, z, w] = self.0;
        [
            [
                1. - 2. * (y * y + z * z),
                2. * (x * y + z * w),
                2. * (x * z - y * w),
            ],
            [
                2. * (x * y - z * w),
                1. - 2. * (x * x + z * z),
                2. * (y * z + x * w),
            ],
            [
                2. * (x * z + y * w),
                2. * (y * z - x * w),
                1. - 2. * (x * x + y * y),
            ],
        ]
    }

    pub fn apply(self, vector: [f64; 3]) -> [f64; 3] {
        let m = self.matrix();
        std::array::from_fn(|r| (0..3).map(|c| m[c][r] * vector[c]).sum())
    }
}

pub(super) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
pub(super) fn unit(value: [f64; 3]) -> Option<[f64; 3]> {
    let length = dot(value, value).sqrt();
    (length.is_finite() && length > 0.).then(|| value.map(|value| value / length))
}
pub(super) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
