-- The `ALL` polarity of the window route's empty-list bound. The bound's spellings are
-- the cross-product {`DISTINCT`, `ALL`} × {`)`, `*`} × {ordinary call, window call},
-- each 42601 on both engines; this cell and the two window-`*` cells
-- (`row_number(DISTINCT *) OVER ()`, `row_number(ALL *) OVER ()`, both measured 42601 on
-- both) were the three with no fixture, and the window-`*` pair reaches the identical
-- parser call this does. Its `DISTINCT` twin
-- (`set_quantifier_distinct_window_empty_arg_list_loud.sql`) is the member that used to
-- answer rows; this one was already 42601 by accident, because pglet knew the keyword
-- nowhere inside a call — so the code that teaches it `ALL` has to keep the refusal
-- rather than start accepting it. 42601 on both.
-- @setup
CREATE TABLE sq_wa (a INT);
INSERT INTO sq_wa VALUES (1), (2), (3);
-- @test
-- @expect-error: syntax error at or near ")"
SELECT row_number(ALL) OVER () FROM sq_wa;
