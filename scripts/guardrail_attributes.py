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
            raw = re.match(r'(br|cr|r)(\#*)"', text[i:])
            if raw:
                start = i + raw.end()
                end = text.find('"' + raw[2], start)
                if end < 0:
                    raise ValueError("unclosed raw string")
                result.append(("string" if raw[1] == "r" else "non_string_literal", text[start:end]))
                i = end + 1 + len(raw[2])
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
        found.extend(meta_expectations(attr))
        i = j
    return found


def arguments(stream):
    """Split one meta argument list, preserving nested syntax and literal token kinds."""
    parts = [[]]
    stack = []
    pairs = {")": "(", "]": "[", "}": "{"}
    for token in stream:
        kind, value = token
        if kind == "punct" and value in "([{":
            stack.append(value)
        elif kind == "punct" and value in ")]}":
            if not stack or stack.pop() != pairs[value]:
                raise ValueError("unbalanced attribute arguments")
        if token == ("punct", ",") and not stack:
            parts.append([])
        else:
            parts[-1].append(token)
    if stack:
        raise ValueError("unclosed attribute arguments")
    if parts and not parts[-1]:
        parts.pop()
    return parts


def meta_expectations(attr):
    # Only unqualified builtin attributes carry this policy. Tool attributes and cfg
    # predicates are not recursively searched for words that resemble suppressions.
    if not attr or attr[0][0] != "word":
        return []
    if attr[1:3] == [("punct", ":"), ("punct", ":")]:
        return []
    name = attr[0][1]
    if name == "allow":
        raise ValueError("allow attribute forbidden, including conditional/multiline forms")
    if name not in ("expect", "cfg_attr"):
        return []
    if attr[1:2] != [("punct", "(")] or attr[-1:] != [("punct", ")")]:
        raise ValueError("attribute requires arguments")
    parts = arguments(attr[2:-1])
    if name == "cfg_attr":
        if len(parts) < 2:
            raise ValueError("cfg_attr needs predicate and attribute")
        return [pair for nested in parts[1:] for pair in meta_expectations(nested)]
    reasons = [part for part in parts if part[:2] == [("word", "reason"), ("punct", "=")]]
    if (len(reasons) != 1 or len(reasons[0]) != 3 or reasons[0][2][0] != "string"
            or not reasons[0][2][1] or not reasons[0][2][1].strip()):
        raise ValueError("expectation needs one literal nonempty reason")
    lints = [part for part in parts if part is not reasons[0]]
    if not lints:
        raise ValueError("expectation needs a lint")
    found = []
    for lint in lints:
        name = "".join(value for kind, value in lint if kind in ("word", "punct"))
        if any(kind not in ("word", "punct") for kind, _ in lint) or not re.fullmatch(
                r"[A-Za-z_][A-Za-z_0-9]*(?:::[A-Za-z_][A-Za-z_0-9]*)*", name):
            raise ValueError("unsupported expectation lint syntax")
        found.append((name, reasons[0][2][1]))
    return found
