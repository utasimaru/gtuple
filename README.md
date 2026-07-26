# gtuple
このマクロは、Rustにおける「複数の異なる型（例：構造体）のインスタンスをまとめたタプルに対して、同じトレイトのメソッドを一括で呼び出せるようにするコード」を自動生成するための手続き的マクロです。  通常、異なる型をまとめたタプル（例：(Robot, Human)）に対して共通の操作をしようとすると、個別に処理を書くか、専用のラッパー構造体を用意する必要があります。このマクロをトレイト定義に付与するだけで、その面倒なボイラープレートコードを完全に自動化できます。

# 主な機能と特徴
## 1. タプル用トレイト（{TraitName}Tuple）の自動生成
付与されたトレイト定義を解析し、戻り値をすべて配列（[T; N]）に変換した新しいタプル用トレイトを自動的に定義します。戻り値があるメソッド：Ret $\rightarrow$ [Ret; N]（配列として一括取得）戻り値がないメソッド（() または省略）：配列化せず、各要素に対して順番に実行（セミコロン区切りで順次呼び出し）
## 2. 指定範囲のタプル実装を一括生成
属性として任意の要素数の範囲（例：#[generate_tuple(2, 6)]）を指定でき、その範囲内のタプル（(T0, T1) から (T0, ..., T5) まで）に対するトレイト実装を動的に展開します。省略時はデフォルトで $N = 1 \sim 12$ が生成されます。
## 3. 高度なシグネチャのサポート複雑なジェネリクス・ライフタイム・where句
元のメソッドが持つ型パラメータや制約をそのまま維持してタプル用トレイト側に伝搬させます。
## 4. サポート外メソッドのスキップ
自動的に、または#[tuple_skip]属性で、非Sized出力のメソッドや、タプル全体へ同時に分配して呼び出すことが不可能なメソッド（静的メソッド、所有権をムーブする値渡しの引数を持つメソッドなど）は、タプル用トレイトの定義および実装から除外します。これにより、コンパイルエラーを防ぎます。

使用例イメージ
```Rust
use gtuple::gtuple;

// 1. トレイトにマクロを付与する (N = 2〜3 のタプル実装を生成)
#[gtuple(2, 3)]
pub trait Worker {
    fn do_work(&self, hours: usize) -> String;
    fn rest(&self);
}

// 2. 異なる型にトレイトを実装
struct Robot;
impl Worker for Robot { /* ... */ }

struct Human;
impl Worker for Human { /* ... */ }

// 3. まとめて呼び出す
fn main() {
    let team = (Robot::new(), Human::new());

    // [String; 2] としてそれぞれの結果が返ってくる！
    let results: [String; 2] = team.do_work(8); 

    // それぞれの rest() が順番に実行される！
    team.rest(); 
}
```
# 原理
バニラコードでも、次のようにトレイトと、トレイトのタプルが実装すべきトレイトを定義することで、一括でトレイトの関数を実行できるようになります。
```Rust
// [step 0] タプルで呼び出せる用にしたいトレイト
pub trait Processor {
    fn execute(&mut self, arg: i32) -> i32;
}
// [step 1] 元トレイトに似せたタプル用トレイトを定義
pub trait ProcessorTuple<const N: usize> {
    fn execute(&mut self, arg: i32) -> [i32; N];
}
// [step 2] このように書くと、タプルから一括で関数を呼び出し、結果を受け取れる(ここでは２個と３個のタプルについて定義)
impl<T0, T1> ProcessorTuple<2> for (T0, T1)
where
    T0: Processor, // step 0 のトレイトを実装した型
    T1: Processor,
{
    fn execute(&mut self, arg: i32) -> [i32; 2] {
        [self.0.execute(arg), self.1.execute(arg)]
    }
}
impl<T0, T1, T2> ProcessorTuple<3> for (T0, T1, T2)
where
    T0: Processor, // step 0 のトレイトを実装した型
    T1: Processor,
    T2: Processor,
{
    fn execute(&mut self, arg: i32) -> [i32; 3] {
        [
            self.0.execute(arg),
            self.1.execute(arg),
            self.2.execute(arg),
        ]
    }
}
```

# マクロ定義はGeminiに丸投げ
私は手続きマクロを書けない(勉強したくない)ので、Geminiに丸投げしました。次のコードを張り付け、「コメントに指定した要件を満たす手続きマクロ"gtuple"を作成してください」
```Rust

#[gtuple(2, 3)]
pub trait Processor<T, W>
where
    W: Sized + Default,
{
    fn t_some(&self, arg: &T) -> W;
    fn lifetime<'a, A>(&self) -> &'a mut A;
    fn num_some(&mut self, num: usize) -> usize;
    fn no_ret(&self);
    fn new(arg: u32); //staticメソッドは、タプル用トレイトに含めない
    fn move_any(&self, vec: Vec<usize>); // 所有権が移動する入力を持つ場合は、タプル用トレイトに含めない
    #[skip_gtuple]
    fn ignore(&self); //skipを指定されたメソッドも、タプル用トレイトに含めない
}
// 上記のようにトレイト定義をマクロに入力すると、そのトレイト定義を行って以下のようなコードを生成する


// 入力された定義に合わせた{入力マクロ名}+Tupleという名前の、入力マクロに似せた「タプル用トレイト」の定義を生成
pub trait ProcessorTuple<const N: usize, T, W>
where
    W: Sized + Default,
{
    fn t_some(&self, arg: &T) -> [W; N];
    fn lifetime<'a, A>(&self) -> [&'a mut A; N];
    fn num_some(&mut self, num: usize) -> [usize; N];
    fn no_ret(&self) -> [(); N];
}
// #[gtuple(2, 3)]で指定された下限と上限の間にあるNに関して、次のようにタプル用トレイトをタプルに実装する
impl<T0, T1, T, W> ProcessorTuple<2, T, W> for (T0, T1)
where
    T0: Processor<T, W>,
    T1: Processor<T, W>,
    W: Sized + Default,
{
    fn t_some(&self, arg: &T) -> [W; 2] {
        [self.0.t_some(arg), self.1.t_some(arg)]
    }
    fn lifetime<'a, A>(&self) -> [&'a mut A; 2] {
        [self.0.lifetime(), self.1.lifetime()]
    }
    fn num_some(&mut self, num: usize) -> [usize; 2] {
        [self.0.num_some(num), self.1.num_some(num)]
    }
    fn no_ret(&self) -> [(); 2] {
        [self.0.no_ret(), self.1.no_ret()]
    }
}
impl<T0, T1, T2, T, W> ProcessorTuple<3, T, W> for (T0, T1, T2)
where
    T0: Processor<T, W>,
    T1: Processor<T, W>,
    T2: Processor<T, W>,
    W: Sized + Default,
{
    fn t_some(&self, arg: &T) -> [W; 3] {
        [self.0.t_some(arg), self.1.t_some(arg), self.2.t_some(arg)]
    }
    fn lifetime<'a, A>(&self) -> [&'a mut A; 3] {
        [self.0.lifetime(), self.1.lifetime(), self.2.lifetime()]
    }
    fn num_some(&mut self, num: usize) -> [usize; 3] {
        [
            self.0.num_some(num),
            self.1.num_some(num),
            self.2.num_some(num),
        ]
    }
    fn no_ret(&self) -> [(); 3] {
        [self.0.no_ret(), self.1.no_ret(), self.2.no_ret()]
    }
}

```
