-- `ALL` is the **no-op** quantifier: 18.6 accepts it on any call that resolves, and
-- `gram.y`'s comment on that production says it would like to mark the node "must be an
-- aggregate" and has nowhere to put the bit. pglet's parser knew the keyword nowhere
-- inside a call, so every spelling was 42601 before this — an over-rejection, and the
-- polarity that catches a fix which refuses both keywords alike.
-- @test
SELECT upper(ALL 'a') AS u, abs(ALL -1) AS n, concat(ALL 'a', 'b') AS c;
