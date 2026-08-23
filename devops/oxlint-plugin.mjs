const ruleMeta = message => ({
  type: 'suggestion',
  docs: {
    description: message
  },
  schema: []
})

const noSemicolonsRule = {
  meta: ruleMeta('Disallow optional semicolons'),
  create(context) {
    const sourceCode = context.sourceCode
    const unsafeClassFieldNames = new Set(['get', 'set', 'static'])
    const unsafeClassFieldFollowers = new Set(['*', 'in', 'instanceof'])

    const isClassFieldHazard = node => {
      if (node.type !== 'PropertyDefinition') return false

      if (!node.computed && node.key.type === 'Identifier' && unsafeClassFieldNames.has(node.key.name)) {
        const isStaticStatic = node.static && node.key.name === 'static'
        if (!isStaticStatic && !node.value) return true
      }

      return unsafeClassFieldFollowers.has(sourceCode.getTokenAfter(node)?.value)
    }

    const canRemoveSemicolon = node => {
      const tokens = sourceCode.getTokens(node)
      const semicolon = tokens.at(-1)
      if (semicolon?.value !== ';') return false

      const nextToken = sourceCode.getTokenAfter(node)
      if (!nextToken || nextToken.value === '}' || nextToken.value === ';') return true
      if (isClassFieldHazard(node)) return false

      const previousToken = tokens.at(-2)
      if (previousToken && previousToken.loc.end.line === nextToken.loc.start.line) return false

      return !/^[-[(/+`]/u.test(nextToken.value) || nextToken.value === '++' || nextToken.value === '--'
    }

    const check = node => {
      if (!canRemoveSemicolon(node)) return
      context.report({ node: sourceCode.getLastToken(node), message: 'Unnecessary semicolon' })
    }

    const checkVariable = node => {
      const parent = node.parent
      if ((parent.type === 'ForStatement' && parent.init === node)
        || (/^For(?:In|Of)Statement$/u.test(parent.type) && parent.left === node)) return
      check(node)
    }

    return {
      VariableDeclaration: checkVariable,
      ExpressionStatement: check,
      ReturnStatement: check,
      ThrowStatement: check,
      DoWhileStatement: check,
      DebuggerStatement: check,
      BreakStatement: check,
      ContinueStatement: check,
      ImportDeclaration: check,
      ExportAllDeclaration: check,
      ExportNamedDeclaration(node) {
        if (!node.declaration) check(node)
      },
      ExportDefaultDeclaration(node) {
        if (!/(?:Class|Function)Declaration$/u.test(node.declaration.type)) check(node)
      },
      PropertyDefinition: check
    }
  }
}

const singleQuotesRule = {
  meta: ruleMeta('Require single quotes for string literals'),
  create(context) {
    return {
      Literal(node) {
        if (typeof node.value === 'string' && context.sourceCode.getText(node).startsWith('"')) {
          context.report({ node, message: 'Strings must use single quotes' })
        }
      },
      TemplateLiteral(node) {
        if (node.expressions.length === 0 && node.parent?.type !== 'TaggedTemplateExpression') {
          context.report({ node, message: 'Strings must use single quotes' })
        }
      }
    }
  }
}

export default {
  meta: {
    name: 'online-dsl-forge'
  },
  rules: {
    'no-semicolons': noSemicolonsRule,
    'single-quotes': singleQuotesRule
  }
}
