-- The enum polarity of `window_derived_table_repeated_name_non_enum_literal.sql`: here the
-- column the window query's qualifier names is the enum. `b.m` is `wte`'s `mood` and the
-- first `m` in the input is `wtt`'s `text`. The window arm's bare-name lookup handed the
-- derived table `wtt`'s descriptor, so `x.m` carried no enum and `'ok' > 'sad'` compared
-- the label bytes (`f`) where 18.6 compares member positions (`t`).
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE wtt (m TEXT NOT NULL);
CREATE TABLE wte (m mood NOT NULL);
INSERT INTO wtt VALUES ('q');
INSERT INTO wte VALUES ('ok');

-- @test
SELECT (SELECT x.m FROM (SELECT b.m, row_number() OVER () AS rn FROM wtt a, wte b) x) > 'sad';
