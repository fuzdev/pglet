-- Between two bad defaults, **declared column order** wins — and the pair is
-- cross-code, which is what makes it a real precedence assertion: `@expect-error`
-- compares SQLSTATE only, so a same-code pair is satisfied by an engine that always
-- answers that code. Here the first column's `22P02` and the second's `22007` swap
-- with the column order. This is the `INT`-first half; `…_date_first` is its mirror.
-- @expect-error: the first declared column's default is cooked first
-- @test
CREATE TABLE def_order (a INT DEFAULT 'x', b DATE DEFAULT 'y');
