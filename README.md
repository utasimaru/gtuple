# gtuple_monomorphization

`gtuple_monomorphization` は、「同じトレイトの複数の異なる型」をタプルにしたときに、一括でメソッドを呼び出せるようにする、トレイト定義につける属性マクロです。

## 特徴

- **完全な静的呼び出し**: コードは完全に静的な呼び出しに変換されます。
- **配列での値収集**: 各要素の戻り値を固定長配列 `[T; N]` にして返します。
- **スキップ機能**: `self` の所有権を奪うメソッドや、固定長配列化できない型を返すメソッド、および `#[skip_gtuple]` が付与されたメソッドをタプル用コードから除外します。
- **柔軟なサイズ範囲**: 生成するタプルの要素数範囲を `#[gtuple(min, max)]` で指定可能です（デフォルトは `2..=12`）。

## インストール

`Cargo.toml` の依存関係に以下を追加してください。

```toml
[dependencies]
gtuple_monomorphization = "0.1.0"
```

# 使い方

以下は、Alphabet トレイトに #[gtuple(2, 3)] を付与し、タプル経由でメソッドを一括実行する例です。

```rust
use gtuple_monomorphization::gtuple;

// 1. マクロを付与してトレイトを定義
#[gtuple(2, 3)]
pub trait Alphabet {
    fn to_char(&self) -> char;
}

// 2. 構造体にトレイトを実装
struct A;
impl Alphabet for A {
    fn to_char(&self) -> char {
        'A'
    }
}

struct B;
impl Alphabet for B {
    fn to_char(&self) -> char {
        'B'
    }
}

fn main() {
    let a = A;
    let b = B;

    // 不変参照のタプルから各要素の to_char を一括実行し、配列として取得
    let chars: [char; 2] = (&a, &b).to_char();
    assert_eq!(chars, ['A', 'B']);
}
```

「マクロ展開後のコード」と同等なコード例は以下の通りです：

```rust
// 1. 元のトレイト
pub trait Alphabet {
    fn to_char(&self) -> char;
}

// 2. マクロによって生成される &self 用のタプルトレイト
//    (戻り値が `char` から配列 `[char; N]` に変化します)
pub trait AlphabetTuple<const N: usize> {
    fn to_char(&self) -> [char; N];
}

// 3. マクロによって生成される &mut self 用のタプルトレイト
pub trait AlphabetMutTuple<const N: usize> {
    fn to_char(&self) -> [char; N];
}

// 4. マクロによって自動生成される実装 (要素数 N=2 の例) ---
impl<'__tuple_macro_lt, T0, T1> AlphabetTuple<2> for (&'__tuple_macro_lt T0, &'__tuple_macro_lt T1)
where
    T0: Alphabet,
    T1: Alphabet,
{
    fn to_char(&self) -> [char; 2] {
        [self.0.to_char(), self.1.to_char()]
    }
}

// --- 構造体の実装 ---
struct A;
impl Alphabet for A {
    fn to_char(&self) -> char { 'A' }
}

struct B;
impl Alphabet for B {
    fn to_char(&self) -> char { 'B' }
}

let a = A;
let b = B;

// タプル参照に対して AlphabetTuple::to_char が呼び出されます
assert_eq!((&a, &b).to_char(), ['A', 'B']);
```

# ライセンス

このプロジェクトは、MITライセンス または Apache License 2.0ライセンス の下で公開されています。
