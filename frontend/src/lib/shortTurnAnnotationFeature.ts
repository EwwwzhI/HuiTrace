export function isShortTurnAnnotationEnabled(
  nodeEnv = process.env.NODE_ENV,
  explicitFlag = process.env.NEXT_PUBLIC_ENABLE_SHORT_TURN_ANNOTATION,
) {
  return nodeEnv === 'development' || explicitFlag === 'true';
}
