//! Futures that join tuples of futures: `zip`, `try_zip`, `race`, `race_biased`.

macro_rules! apply_on_tuples {
    ($macro:ident!) => {
        #[allow(non_snake_case)]
        const _: () = {
            $macro! {                         A, B                                                 }
            $macro! {                        A, B, C                                               }
            $macro! {                       A, B, C, D                                             }
            $macro! {                      A, B, C, D, E                                           }
            $macro! {                     A, B, C, D, E, F                                         }
            $macro! {                    A, B, C, D, E, F, G                                       }
            $macro! {                   A, B, C, D, E, F, G, H                                     }
            $macro! {                  A, B, C, D, E, F, G, H, I                                   }
            $macro! {                 A, B, C, D, E, F, G, H, I, J                                 }
            $macro! {                A, B, C, D, E, F, G, H, I, J, K                               }
            $macro! {               A, B, C, D, E, F, G, H, I, J, K, L                             }
            $macro! {              A, B, C, D, E, F, G, H, I, J, K, L, M                           }
            $macro! {             A, B, C, D, E, F, G, H, I, J, K, L, M, N                         }
            $macro! {            A, B, C, D, E, F, G, H, I, J, K, L, M, N, O                       }
            $macro! {           A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P                     }
            $macro! {          A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q                   }
            $macro! {         A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R                 }
            $macro! {        A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S               }
            $macro! {       A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T             }
            $macro! {      A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U           }
            $macro! {     A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V         }
            $macro! {    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W       }
            $macro! {   A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X     }
            $macro! {  A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y   }
            $macro! { A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z }
        };
    };
}

mod base;

mod zip;
pub use zip::{zip, Zip};
