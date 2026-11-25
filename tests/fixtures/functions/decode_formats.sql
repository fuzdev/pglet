-- `decode` in its three formats, the name matched case-blind: hex with whitespace between
-- pairs, base64 with padding and whitespace, and the escape format's `\\` and `\ooo`.
-- @test
SELECT decode('deadbeef', 'hex') AS h, decode('DEAD BEEF', 'HEX') AS h2, decode('', 'hex') AS empty,
       decode('aGVsbG8=', 'base64') AS b, decode('aG k=', 'base64') AS b_space,
       decode('aGk=QQ==', 'base64') AS b_after_pad, decode('a\001x\\y', 'escape') AS e,
       decode('abc', 'Escape') AS e2, decode(NULL, 'hex') AS nul;
