"""Conservative AST admission for repository-importing Python entrypoints."""
import ast


def require_bytecode_guard(source, local_modules):
    """Conservative AST admission for repository-module import entrypoints."""
    tree = ast.parse(source)
    sys_names = {'sys'}
    import_ops = {'spec_from_file_location', 'spec_from_loader', 'module_from_spec',
                  'exec_module', 'import_module', 'SourceFileLoader', 'run_module'}
    loader_names = {'__import__', *import_ops}
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            sys_names.update(a.asname or a.name for a in node.names if a.name == 'sys')
            loader_names.update(a.asname or a.name.split('.')[0] for a in node.names
                                if a.name.split('.')[0] in ('importlib', 'runpy'))
        if isinstance(node, ast.ImportFrom) and ((node.module or '').startswith('importlib')
                                               or node.module == 'runpy'):
            loader_names.update(a.asname or a.name for a in node.names)
    hazards = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Attribute) and (node.attr in import_ops or
                (node.attr == 'path' and isinstance(node.value, ast.Name) and node.value.id in sys_names)):
            hazards.append(node.lineno)
        if isinstance(node, ast.Name) and isinstance(node.ctx, ast.Load) and node.id in loader_names:
            hazards.append(node.lineno)
        if isinstance(node, ast.Import) and any(a.name.split('.')[0] in local_modules for a in node.names):
            hazards.append(node.lineno)
        if isinstance(node, ast.ImportFrom) and (node.level or
                (node.module or '').split('.')[0] in local_modules or
                (node.module == 'sys' and any(a.name == 'path' for a in node.names))):
            hazards.append(node.lineno)
    if not hazards:
        return
    guards = []
    for node in ast.walk(tree):
        if isinstance(node, (ast.Assign, ast.AnnAssign, ast.AugAssign)):
            targets = node.targets if isinstance(node, ast.Assign) else [node.target]
            if any(isinstance(t, ast.Attribute) and t.attr == 'dont_write_bytecode' for t in targets):
                valid = (isinstance(node, ast.Assign) and len(targets) == 1
                         and isinstance(targets[0].value, ast.Name) and targets[0].value.id in sys_names
                         and isinstance(node.value, ast.Constant) and node.value.value is True)
                if not valid:
                    raise ValueError('PY_BYTECODE_GUARD: bytecode guard reset or nonliteral assignment')
                if node in tree.body:
                    guards.append(node.lineno)
    if not guards or min(guards) >= min(hazards):
        raise ValueError('PY_BYTECODE_GUARD: repo imports need unconditional prior sys.dont_write_bytecode = True')

