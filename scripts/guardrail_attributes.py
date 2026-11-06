"""Conservative Rust attribute lexer for the expectation ledger; not a Rust type checker."""
import json
import re


def tokens(text):
    result = []
    i = 0
    while i < len(text):
        if text[i].isspace():
            i += 1
        elif text.startswith("//", i):
            end = text.find("\n", i)
            i = len(text) if end < 0 else end + 1
        elif text.startswith("/*", i):
            depth = 1
            i += 2
            while depth and i < len(text):
                if text.startswith("/*", i):
                    depth += 1
                    i += 2
                elif text.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
            if depth:
                raise ValueError("unclosed Rust block comment")
        else:
            raw = re.match(r'(?:br|cr|r)(\#*)"', text[i:])
            if raw:
                start = i + raw.end()
                end = text.find('"' + raw[1], start)
                if end < 0:
                    raise ValueError("unclosed raw string")
                result.append(("string", text[start:end]))
                i = end + 1 + len(raw[1])
                continue
            if text[i] == '"':
                start = i
                i += 1
                while i < len(text) and text[i] != '"':
                    i += 2 if text[i] == "\\" else 1
                if i >= len(text):
                    raise ValueError("unclosed string")
                literal = text[start:i + 1]
                # Ordinary Rust strings outside attributes may use non-JSON escapes; retain
                # a marker instead of interpreting them. Reasons with such escapes refuse below.
                try:
                    value = json.loads(literal)
                except ValueError:
                    value = None
                result.append(("string", value))
                i += 1
                continue
            char = re.match(r"'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\])'", text[i:])
            if char:
                result.append(("char", char[0]))
                i += char.end()
                continue
            word = re.match(r"(?:r#)?[A-Za-z_][A-Za-z_0-9]*", text[i:])
            if word:
                result.append(("word", word[0].removeprefix("r#")))
                i += word.end()
            else:
                result.append(("punct", text[i]))
                i += 1
    return result


def expectations(text):
    stream = tokens(text)
    found = []
    i = 0
    while i < len(stream):
        if stream[i] != ("punct", "#"):
            i += 1
            continue
        j = i + 1
        if j < len(stream) and stream[j] == ("punct", "!"):
            j += 1
        if j >= len(stream) or stream[j] != ("punct", "["):
            i += 1
            continue
        start = j + 1
        depth = 1
        j += 1
        while j < len(stream) and depth:
            if stream[j] == ("punct", "["):
                depth += 1
            elif stream[j] == ("punct", "]"):
                depth -= 1
            j += 1
        if depth:
            raise ValueError("unclosed attribute")
        attr = stream[start:j - 1]
        for n, token in enumerate(attr):
            if token == ("word", "allow") and attr[n + 1:n + 2] == [("punct", "(")]:
                raise ValueError("allow attribute forbidden, including conditional/multiline forms")
            if token != ("word", "expect") or attr[n + 1:n + 2] != [("punct", "(")]:
                continue
            args = []
            k = n + 2
            while k < len(attr) and attr[k] != ("punct", ")"):
                args.append(attr[k])
                k += 1
            if k == len(attr):
                raise ValueError("unclosed expectation")
            parts = [[]]
            for item in args:
                if item == ("punct", ","):
                    parts.append([])
                else:
                    parts[-1].append(item)
            reasons = [part for part in parts if part[:2] == [("word", "reason"), ("punct", "=")]]
            if len(reasons) != 1 or len(reasons[0]) != 3 or reasons[0][2][0] != "string" or not reasons[0][2][1]:
                raise ValueError("expectation needs one literal nonempty reason")
            reason = reasons[0][2][1]
            lints = [part for part in parts if part and part is not reasons[0]]
            if not lints:
                raise ValueError("expectation needs a lint")
            for lint in lints:
                name = "".join(value for kind, value in lint if kind in ("word", "punct"))
                if not re.fullmatch(r"[A-Za-z_][A-Za-z_0-9]*(?:::[A-Za-z_][A-Za-z_0-9]*)*", name):
                    raise ValueError("unsupported expectation lint syntax")
                found.append((name, reason))
        i = j
    return found
