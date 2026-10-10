-- Which error wins is decided row by row, in list order. 18.6 transforms the first row
-- whole — its cells, its length, then each cell's assignment to its column, where the
-- unknown literal `'x'` meets `integer`'s input function — before it reads the second row,
-- so the first row's `22P02` wins over the second row's length. A length pass over the whole
-- list ahead of the rows answers `42601` here.
-- @setup
CREATE TABLE vlel (a INT, b INT);

-- @test
-- @expect-error: invalid input syntax for type integer: "x" (22P02), ahead of the second row's 42601
-- @expect-message
INSERT INTO vlel VALUES (1, 'x'), (2);
