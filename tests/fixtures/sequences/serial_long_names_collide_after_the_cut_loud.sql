-- Two `SERIAL` columns whose names differ only past the cut choose the **same** sequence
-- name — each is chosen against the committed catalog, before either sequence exists — so
-- the second `CREATE SEQUENCE` is `42P07` on 18.6. An engine that does not cut the name
-- creates two sequences and accepts the table.
-- @test
CREATE TABLE sln2_ttttttttttttttttttttttttttttttttttt (cooooooooooooooooooooooooooooooooooooooo SERIAL, cooooooooooooooooooooooooooooooooooooooox SERIAL);
-- @expect-error: relation "…_seq" already exists (42P07)
